// The errors adapter gives what GET /api/errors gave, on real rows of `journals.errors`: the fixture holds the rows (with
// their total) and the endpoint's page for the same page and size, captured on integration-tests/query-zhang-tables.
// Run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { type LedgerErrorRow, ledgerErrorOf, ledgerErrorPage } from './ledger-errors.ts';

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
  total: number;
}

function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

const fixture = JSON.parse(await readFile(new URL('./ledger-errors.fixture.json', import.meta.url), 'utf8'));

test('a page of errors is what GET /api/errors answered for the same page and size', () => {
  const pages = Object.entries<{ size: number; page: number; rows: Result; expected: unknown }>(fixture.pages);
  assert.equal(pages.length, 3);
  for (const [name, { size, page, rows: result, expected }] of pages) {
    assert.deepEqual(ledgerErrorPage(rows<LedgerErrorRow>(result), result.total, page, size), expected, name);
  }
  // the first page lists errors of several kinds, with and without metas
  const first = fixture.pages['page 1 of 50'].expected;
  assert.equal(first.records.length, 6);
  assert.ok(first.records.some((it: { metas: object }) => Object.keys(it.metas).length > 0));
});

test('an error without a known start has no span, and a last meta value wins', () => {
  const row: LedgerErrorRow = {
    id: 'x',
    kind: 'PluginError',
    file: null,
    line: null,
    column: null,
    span_start: null,
    span_end: null,
    source: null,
    metas: [
      { key: 'a', value: '1' },
      { key: 'a', value: '2' },
    ],
  };
  assert.deepEqual(ledgerErrorOf(row), { id: 'x', span: null, error_type: 'PluginError', metas: { a: '2' } });
  // an end the row does not know is the start
  assert.deepEqual(ledgerErrorOf({ ...row, span_start: 10, span_end: null, source: 'x', file: 'main.zhang', line: 3, column: 1, metas: [] }).span, {
    start: 10,
    end: 10,
    content: 'x',
    filename: 'main.zhang',
    line: 3,
    column: 1,
  });
});

test('the page count rounds up, and no errors are no pages', () => {
  assert.equal(ledgerErrorPage([], 0, 1, 10).total_page, 0);
  assert.equal(ledgerErrorPage([], 10, 1, 10).total_page, 1);
  assert.equal(ledgerErrorPage([], 11, 2, 10).total_page, 2);
});
