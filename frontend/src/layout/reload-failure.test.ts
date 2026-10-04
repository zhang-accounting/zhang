// Tests for the failed-reload notice (#492), run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { reloadFailureDetail, reloadFailureEditorHref } from './reload-failure.ts';

const syntaxError = {
  file: 'main.zhang',
  message: 'cannot parse main.zhang: failed to parse zhang file: unexpected input at line 3, column 1',
};

test('the notice shows the reason, which already names the file for a syntax error', () => {
  assert.equal(reloadFailureDetail(syntaxError), syntaxError.message);
});

test('the notice names the file when the reason does not', () => {
  assert.equal(reloadFailureDetail({ file: 'data/2024/01.zhang', message: 'plugin fx failed: timeout' }), 'data/2024/01.zhang: plugin fx failed: timeout');
  const outside = 'cannot include /elsewhere/x.zhang: it is outside the ledger';
  assert.equal(reloadFailureDetail({ file: null, message: outside }), outside);
  assert.equal(reloadFailureDetail({ message: 'panic on reload: boom' }), 'panic on reload: boom');
});

test('the notice links to the raw editor, opened on the failing file when it is in the ledger', () => {
  assert.equal(reloadFailureEditorHref(syntaxError), '/edit?file=main.zhang');
  assert.equal(reloadFailureEditorHref({ file: 'data/2024/01.zhang', message: 'x' }), '/edit?file=data%2F2024%2F01.zhang');
  // a file the editor cannot list, or none: the editor itself
  assert.equal(reloadFailureEditorHref({ file: '/private/var/ledger/main.zhang', message: 'x' }), '/edit');
  assert.equal(reloadFailureEditorHref({ file: null, message: 'x' }), '/edit');
  assert.equal(reloadFailureEditorHref({ message: 'x' }), '/edit');
});
