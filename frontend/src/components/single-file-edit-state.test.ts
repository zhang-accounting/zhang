// Tests of the raw editor's save outcomes, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { type EditorAction, editorReducer, initialEditorState, isConflict, isDirty } from './single-file-edit-state.ts';

const ORIGINAL = '2024-01-01 open Assets:Cash\n';
const EDITED = `${ORIGINAL}2024-01-02 open Assets:Bank\n`;

function run(...actions: EditorAction[]) {
  return actions.reduce(editorReducer, initialEditorState);
}

const opened = (): EditorAction[] => [{ type: 'loaded', content: ORIGINAL, sha256: 'abc' }, { type: 'edited', content: EDITED }, { type: 'save_started' }];

test('a save refused because the file changed shows the conflict and does not claim success', () => {
  const state = run(...opened(), { type: 'save_refused' });
  assert.equal(state.conflict, true);
  assert.equal(state.saving, false);
  assert.equal(state.content, EDITED, 'the buffer is kept');
  assert.equal(state.saved, ORIGINAL, 'nothing counts as saved');
  assert.equal(isDirty(state), true, 'the edit is still unsaved');
  assert.equal(state.sha256, 'abc', 'the stale fingerprint is kept, so the next save is checked again');
});

test('keeping editing after a refused save keeps the buffer unsaved', () => {
  const state = run(...opened(), { type: 'save_refused' }, { type: 'keep_editing' });
  assert.equal(state.conflict, false);
  assert.equal(state.content, EDITED);
  assert.equal(state.saved, ORIGINAL);
  assert.equal(isDirty(state), true);
});

test('reloading after a refused save replaces the buffer with the file as it is now', () => {
  const now = `${ORIGINAL}2024-01-03 open Income:Salary\n`;
  const state = run(...opened(), { type: 'save_refused' }, { type: 'loaded', content: now, sha256: 'def' });
  assert.deepEqual(state, { content: now, saved: now, sha256: 'def', saving: false, conflict: false });
  assert.equal(isDirty(state), false);
});

test('a written save takes the file as read back, with its new fingerprint', () => {
  const state = run(...opened(), { type: 'saved', saved: EDITED, sha256: 'def' });
  assert.deepEqual(state, { content: EDITED, saved: EDITED, sha256: 'def', saving: false, conflict: false });
  assert.equal(isDirty(state), false);
});

test('a written save whose fingerprint could not be read back keeps the old one, so the next save is checked', () => {
  const state = run(...opened(), { type: 'saved', saved: EDITED, sha256: null });
  assert.equal(state.sha256, 'abc');
  assert.equal(state.saved, EDITED);
});

test('a save that failed for another reason is neither a conflict nor saved', () => {
  const state = run(...opened(), { type: 'save_failed' });
  assert.equal(state.conflict, false);
  assert.equal(state.saving, false);
  assert.equal(state.saved, ORIGINAL);
  assert.equal(isDirty(state), true);
});

test('only a 409 is a conflict', () => {
  assert.equal(isConflict({ status: 409, data: { message: 'the file changed' } }), true);
  assert.equal(isConflict(new Response(null, { status: 409 })), true);
  assert.equal(isConflict({ status: 500 }), false);
  assert.equal(isConflict(new Error('network')), false);
  assert.equal(isConflict(undefined), false);
});
