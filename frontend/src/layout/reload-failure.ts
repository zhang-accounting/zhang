/**
 * The failure of the last reload, as `/api/info` (`reload_failure`) and the SSE `ReloadFailed` event carry it: the server
 * keeps serving the ledger loaded before it until a reload succeeds (#492).
 */
export interface ReloadFailure {
  /** The file the failure is in, when it is one file's (a syntax error names its file), as the ledger names its files. */
  file?: string | null;
  message: string;
}

/**
 * What the notice says: the message, prefixed with the file when the failure names one that the message does not already
 * mention (a syntax error's message names its file, a plugin's may not).
 */
export function reloadFailureDetail(failure: ReloadFailure): string {
  const file = failure.file?.trim();
  if (file && !failure.message.includes(file)) return `${file}: ${failure.message}`;
  return failure.message;
}

/**
 * Where to fix it: the raw editor opened on the failing file when the failure names one the editor lists (a path relative to
 * the ledger root), otherwise the editor itself.
 */
export function reloadFailureEditorHref(failure: ReloadFailure): string {
  const file = failure.file?.trim();
  if (file && !file.startsWith('/')) return `/edit?file=${encodeURIComponent(file)}`;
  return '/edit';
}
