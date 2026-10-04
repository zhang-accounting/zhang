/**
 * The state of the raw editor of one file (`SingleFileEdit`), kept apart from the component so the save outcomes can be
 * tested without a DOM: the buffer, the file as last loaded or saved, and whether a save was refused because the file
 * changed since it was loaded.
 */
export interface EditorState {
  /** the editor's buffer */
  content: string;
  /** the file as the editor loaded or saved it last; `null` until it is loaded */
  saved: string | null;
  /**
   * the fingerprint the server served `saved` with (`sha256` of `GET /api/files/{path}`), sent back with a save as
   * `expected_sha256` so the server refuses to overwrite a file that changed since; `null` until the file is loaded
   */
  sha256: string | null;
  saving: boolean;
  /**
   * a save was refused (409) because the file changed since the editor loaded it. The buffer is kept, and stays
   * unsaved, until the user reloads the file or keeps editing
   */
  conflict: boolean;
}

export type EditorAction =
  /** the file was loaded, or reloaded on the user's request: the buffer is replaced */
  | { type: 'loaded'; content: string; sha256: string }
  | { type: 'edited'; content: string }
  | { type: 'save_started' }
  /** the save was written: `saved` is the file as read back after it, with its new fingerprint when it could be read */
  | { type: 'saved'; saved: string; sha256: string | null }
  /** the server refused the save: the file changed since it was loaded, and nothing was written */
  | { type: 'save_refused' }
  | { type: 'save_failed' }
  /** the user keeps the buffer after a refused save; the next save is checked again */
  | { type: 'keep_editing' };

export const initialEditorState: EditorState = { content: '', saved: null, sha256: null, saving: false, conflict: false };

export function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case 'loaded':
      return { content: action.content, saved: action.content, sha256: action.sha256, saving: false, conflict: false };
    case 'edited':
      return { ...state, content: action.content };
    case 'save_started':
      return { ...state, saving: true };
    case 'saved':
      // a fingerprint that could not be read back is kept as it was: at worst the next save is refused and reloads
      return { ...state, saving: false, conflict: false, saved: action.saved, sha256: action.sha256 ?? state.sha256 };
    case 'save_refused':
      // the buffer and what was last saved stay as they are: the edit is still unsaved
      return { ...state, saving: false, conflict: true };
    case 'save_failed':
      return { ...state, saving: false };
    case 'keep_editing':
      return { ...state, conflict: false };
  }
}

/** Whether the buffer differs from the file as last loaded or saved. */
export function isDirty(state: EditorState): boolean {
  return state.saved !== null && state.content !== state.saved;
}

/** Whether a failed request was refused with 409: the file changed since the editor loaded it (the generated client's `ApiError`, or a `Response`). */
export function isConflict(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'status' in error && (error as { status: unknown }).status === 409;
}
