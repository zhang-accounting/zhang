// Pure-function tests for the document helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
// The fixture holds the rows of `journals.documents` and `accounts.documents` and what GET /api/documents and
// GET /api/accounts/{a}/documents answered for the same ledger (zhang-server/tests/fixtures/journals/probe): the adapter
// must give the endpoints' values.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { type DocumentRow, canPreview, canPreviewPath, documentOf, documentType } from './documents.ts';

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
}

function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

/** The MIME types the endpoints gave (from the file extension) that the lists previewed as images. */
const PREVIEWED_MIME_TYPES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif', 'image/bmp']);

const fixture = JSON.parse(await readFile(new URL('./documents.fixture.json', import.meta.url), 'utf8'));

/** The adapter's documents equal the endpoint's, field by field; the endpoint's `mime_type` only decided the preview. */
function expectSame(result: Result, expected: Record<string, unknown>[], where: string) {
  const documents = rows<DocumentRow>(result).map(documentOf);
  const shown = expected.map(({ mime_type, ...rest }) => {
    assert.equal(
      typeof mime_type === 'string' ? PREVIEWED_MIME_TYPES.has(mime_type) : false,
      canPreview(documentOf({ ...rowOf(rest) })),
      `${where}: ${rest.path}`,
    );
    return rest;
  });
  assert.deepEqual(documents, shown, where);
}

/** A row with the path of an expected document, to ask the adapter about its preview. */
function rowOf(expected: Record<string, unknown>): DocumentRow {
  return { date: null, time: null, path: expected.path as string, account: null, transaction_id: null };
}

test('the documents list is what GET /api/documents answered', () => {
  const { rows: result, expected } = fixture.documents;
  assert.ok(expected.length >= 4);
  expectSame(result, expected, '/api/documents');
});

test("an account's documents are what GET /api/accounts/{a}/documents answered, for every account", () => {
  const accounts = Object.entries<{ rows: Result; expected: Record<string, unknown>[] }>(fixture.accountDocuments);
  assert.ok(accounts.length >= 12);
  assert.ok(accounts.filter(([, it]) => it.expected.length > 0).length >= 3, 'accounts with documents');
  for (const [account, { rows: result, expected }] of accounts) {
    expectSame(result, expected, account);
  }
});

test('a document has the date and time, file name and extension of its row', () => {
  assert.deepEqual(documentOf({ date: '2024-01-16', time: '09:30:00', path: 'statements/Jan.PDF', account: 'Assets:Bank', transaction_id: null }), {
    datetime: '2024-01-16T09:30:00',
    filename: 'Jan.PDF',
    path: 'statements/Jan.PDF',
    extension: 'pdf',
    account: 'Assets:Bank',
    trx_id: null,
  });
  // a file name without an extension, and a transaction's document
  assert.deepEqual(documentOf({ date: '2024-01-03', time: null, path: 'notes/README', account: null, transaction_id: 'abc' }), {
    datetime: '2024-01-03T00:00:00',
    filename: 'README',
    path: 'notes/README',
    extension: null,
    account: null,
    trx_id: 'abc',
  });
});

test('the type of a document is the extension of its file name, upper case', () => {
  assert.equal(documentType({ extension: 'pdf' }), 'PDF');
  assert.equal(documentType({ extension: 'png' }), 'PNG');
  assert.equal(documentType({ extension: null }), '');
});

test('a document is previewed by the extension of its file name', () => {
  for (const extension of ['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp']) {
    assert.equal(canPreview({ extension }), true, extension);
  }
  // a browser does not show these in an <img> from the download endpoint, which sends no content type
  for (const extension of ['pdf', 'svg', 'tiff', 'heic', 'txt']) {
    assert.equal(canPreview({ extension }), false, extension);
  }
  assert.equal(canPreview({ extension: null }), false);
});

test('a path is previewed by the extension of its file name, for the same formats', () => {
  assert.equal(canPreviewPath('receipts/a.png'), true);
  assert.equal(canPreviewPath('receipts/A.JPG'), true);
  assert.equal(canPreviewPath('receipts/scan.jpeg'), true);
  assert.equal(canPreviewPath('receipts/photo.webp'), true);
  assert.equal(canPreviewPath('receipts/a.pdf'), false);
  assert.equal(canPreviewPath('receipts/vector.svg'), false);
  // the extension of the file name, not of a directory
  assert.equal(canPreviewPath('photos.png/readme'), false);
  assert.equal(canPreviewPath('receipts/.png'), false);
  assert.equal(canPreviewPath('png'), false);
});
