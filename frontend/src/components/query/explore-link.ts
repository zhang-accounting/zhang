/**
 * "Open in Explore": links to the Query page (`/explore`) with a query in the editor, and the parameter values of the
 * built-in queries (`POST /api/query/builtins/{name}/text`) the pages send to get one written out.
 */

/** The search parameter of `/explore` that holds a query to put in the editor and run. */
export const EXPLORE_QUERY_PARAM = 'query';

/** The Query page with `query` in the editor, run on arrival. */
export function exploreUrl(query: string): string {
  return `/explore?${new URLSearchParams({ [EXPLORE_QUERY_PARAM]: query })}`;
}

/** The query a `/explore` URL carries (`location.search` or its parsed parameters); `null` when there is none. */
export function queryFromSearch(search: string | URLSearchParams): string | null {
  const query = (typeof search === 'string' ? new URLSearchParams(search) : search).get(EXPLORE_QUERY_PARAM);
  return query === null || query.trim() === '' ? null : query;
}

/** A parameter value as a page has it: a `Date` (a day of the ledger's calendar, as the page shows it), a set of strings, ... */
export type BuiltinParamInput = string | number | boolean | null | Date | readonly string[] | ReadonlySet<string>;

/** A parameter value as the server takes it: dates are `YYYY-MM-DD` and sets lists of strings. */
export type BuiltinParamValue = string | number | boolean | null | string[];

/** The day a `Date` shows in the browser, as a ledger date `YYYY-MM-DD`. */
export function ledgerDate(date: Date): string {
  const pad = (value: number, width: number) => String(value).padStart(width, '0');
  return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1, 2)}-${pad(date.getDate(), 2)}`;
}

/** The `params` of `POST /api/query/builtins/{name}/text` from the values a page ran a built-in query with. */
export function toBuiltinParams(params: Record<string, BuiltinParamInput>): Record<string, BuiltinParamValue> {
  return Object.fromEntries(
    Object.entries(params).map(([name, value]) => {
      if (value instanceof Date) return [name, ledgerDate(value)];
      if (value instanceof Set) return [name, [...value]];
      if (Array.isArray(value)) return [name, [...value]];
      return [name, value as string | number | boolean | null];
    }),
  );
}
