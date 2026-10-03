// Pure-function tests for the query cell helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isMetas } from './values.ts';

test('metas cells are lists of key/value pairs', () => {
  assert.equal(isMetas([]), true);
  assert.equal(
    isMetas([
      { key: 'invoice', value: 'a.pdf' },
      { key: 'invoice', value: 'b.pdf' },
    ]),
    true,
  );
  // a set cell is a list of strings, an inventory an object
  assert.equal(isMetas(['a', 'b']), false);
  assert.equal(isMetas({ positions: [] }), false);
  assert.equal(isMetas([{ key: 'x' }]), false);
  assert.equal(isMetas(null), false);
});
