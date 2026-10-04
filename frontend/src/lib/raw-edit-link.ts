/**
 * Deep links into the Raw Editing page (`/edit`): `?file=` names the ledger file to open and `?line=` the line to put
 * the cursor on. The error list opens the file of an error at its directive this way (#493): the fix is often elsewhere
 * in the file, such as an `open` that is missing or a commodity that is not declared, so the editor is where it is made.
 */

export const RAW_EDIT_URI = '/edit';

/** The Raw Editing page on `file`, with the cursor on `line` (1-based) when it is known; `null` when there is no file to open. */
export function rawEditLink(file: string | null | undefined, line?: number | null): string | null {
  if (!file) return null;
  const params = new URLSearchParams({ file });
  if (line != null) params.set('line', String(line));
  return `${RAW_EDIT_URI}?${params}`;
}

/** The line a `/edit` URL asks for (`location.search` or its parsed parameters): a positive whole number, else `null`. */
export function lineFromSearch(search: string | URLSearchParams): number | null {
  const value = (typeof search === 'string' ? new URLSearchParams(search) : search).get('line');
  if (value === null || !/^\d+$/.test(value)) return null;
  const line = Number(value);
  return line >= 1 ? line : null;
}
