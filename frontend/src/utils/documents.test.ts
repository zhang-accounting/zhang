// Pure-function tests for the document helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { canPreview, canPreviewPath, documentType } from './documents.ts';

test('the type of a document is the extension the server gives it, upper case', () => {
  assert.equal(documentType({ extension: 'pdf' }), 'PDF');
  assert.equal(documentType({ extension: 'png' }), 'PNG');
  // never the MIME type, which an older server sent as `extension`, nor a guess from the file name
  assert.equal(documentType({ extension: null }), '');
  assert.equal(documentType({}), '');
});

test('a document is previewed by the MIME type the server gives it', () => {
  for (const mime_type of ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif', 'image/bmp']) {
    assert.equal(canPreview({ mime_type }), true, mime_type);
  }
  // a browser does not show these in an <img> from the download endpoint, which sends no content type
  for (const mime_type of ['application/pdf', 'image/svg+xml', 'image/tiff', 'image/heic', 'text/plain']) {
    assert.equal(canPreview({ mime_type }), false, mime_type);
  }
  assert.equal(canPreview({ mime_type: null }), false);
  assert.equal(canPreview({}), false);
});

test('a path without a MIME type is previewed by the extension of its file name, for the same formats', () => {
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
