import { Buffer } from 'buffer';
import { ApiError } from 'openapi-typescript-fetch';
import { responseError } from '@/lib/api-error';
import { rowsByColumn } from './builtin-rows';
import type { Builtins } from './builtins';
import { apiBaseUrl, openAPIFetcher, reportUnauthorized } from './fetcher';

export const retrieveJournals = openAPIFetcher.path('/api/journals').method('get').create();

export const retrieveStatisticGraph = openAPIFetcher.path('/api/statistic/graph').method('get').create();

export const retrieveFiles = openAPIFetcher.path('/api/files').method('get').create();

export const retrieveOptions = openAPIFetcher.path('/api/options').method('get').create();

/** A ledger option from `/api/options`, trimmed; `undefined` when it is unset or blank. The first one wins if it is set twice. */
export function optionValue(options: { key: string; value: string }[] | undefined, key: string): string | undefined {
  return options?.find((option) => option.key === key)?.value.trim() || undefined;
}

export const retrievePlugins = openAPIFetcher.path('/api/plugins').method('get').create();

export const retrieveAccountInfo = openAPIFetcher.path('/api/accounts/{account_name}').method('get').create();

export const retrieveAccountJournals = openAPIFetcher.path('/api/accounts/{account_name}/journals').method('get').create();

export const retrieveCommodityInfo = openAPIFetcher.path('/api/commodities/{commodity_name}').method('get').create();

export const retrieveNewTransactionInfo = openAPIFetcher.path('/api/for-new-transaction').method('get').create();

export const retrieveNewDocumentInfo = openAPIFetcher.path('/api/for-new-document').method('get').create();

/** A ledger path as the API's path parameters take it (`/api/files/{file_path}`, `/api/documents/{path}`): base64 of its UTF-8 bytes. */
export function base64Path(path: string): string {
  return Buffer.from(path).toString('base64');
}

export const retrieveFile = openAPIFetcher.path('/api/files/{file_path}').method('get').create();

export const updateFile = openAPIFetcher.path('/api/files/{file_path}').method('put').create();

export const createNewTransaction = openAPIFetcher.path('/api/transactions').method('post').create();

export const createBatchBalance = openAPIFetcher.path('/api/accounts/batch-balances').method('post').create();

export const reloadLedger = openAPIFetcher.path('/api/reload').method('post').create();

export const updateTransaction = openAPIFetcher.path('/api/transactions/{transaction_id}').method('put').create();

/** What creating the transaction would write, without writing it: its text, the fields refused, and the ledger's verdict. */
export const previewNewTransaction = openAPIFetcher.path('/api/transactions/preview').method('post').create();

/** What updating the transaction would write, without writing it, as `previewNewTransaction` tells it for a new one. */
export const previewTransactionUpdate = openAPIFetcher.path('/api/transactions/{transaction_id}/preview').method('post').create();

export const createAccountBalance = openAPIFetcher.path('/api/accounts/{account_name}/balances').method('post').create();

export const executeQuery = openAPIFetcher.path('/api/query').method('post').create();

export const retrieveQuerySchema = openAPIFetcher.path('/api/query/schema').method('get').create();

export const retrieveSavedQueries = openAPIFetcher.path('/api/query/saved').method('get').create();

export const retrieveBuiltinQueryText = openAPIFetcher.path('/api/query/builtins/{name}/text').method('post').create();

const postBuiltinQuery = openAPIFetcher.path('/api/query/builtins/{name}').method('post').create();
const postBuiltinQueries = openAPIFetcher.path('/api/query/builtins').method('post').create();

/** The rows of the built-in query `N`, each by column name, as `builtins.ts` types them; any cell may be `null`. */
export type BuiltinRows<N extends keyof Builtins> = Builtins[N]['row'][];

/** A built-in query to run in a batch (`runBuiltins`): its name and the value of every parameter. */
export type BuiltinRun<N extends keyof Builtins = keyof Builtins> = { [K in N]: { name: K; params: Builtins[K]['params']; count_total?: boolean } }[N];

/** The rows of each query of a batch, in the order of the batch. */
export type BuiltinBatchRows<R extends readonly BuiltinRun[]> = {
  -readonly [I in keyof R]: R[I] extends { name: infer N extends keyof Builtins } ? BuiltinRows<N> : never;
};

/**
 * Runs the built-in query `name` (`POST /api/query/builtins/{name}`) with `params` bound, and returns its rows by column
 * name. Pass the same `name` and `params` to `OpenInExplore`, so what the page shows and what opens on the Query page
 * never disagree.
 */
export async function runBuiltin<N extends keyof Builtins>(name: N, params: Builtins[N]['params']): Promise<BuiltinRows<N>> {
  const { columns, rows } = (await postBuiltinQuery({ name, params })).data.data;
  return rowsByColumn<Builtins[N]['row']>(columns, rows);
}

/** `runBuiltin`, also counting the rows before `LIMIT` and `OFFSET` into `total`, for a page of a query. */
export async function runBuiltinWithTotal<N extends keyof Builtins>(name: N, params: Builtins[N]['params']): Promise<{ rows: BuiltinRows<N>; total: number }> {
  const { columns, rows, total } = (await postBuiltinQuery({ name, params, count_total: true })).data.data;
  return { rows: rowsByColumn<Builtins[N]['row']>(columns, rows), total: total ?? rows.length };
}

/**
 * Runs several built-in queries under one read of the ledger (`POST /api/query/builtins`), so the figures of one page agree
 * with each other: the rows of each query, in the order given.
 */
export async function runBuiltins<const R extends readonly BuiltinRun[]>(runs: R): Promise<BuiltinBatchRows<R>> {
  const results = (await postBuiltinQueries([...runs])).data.data;
  return results.map(({ columns, rows }) => rowsByColumn(columns, rows)) as BuiltinBatchRows<R>;
}

/**
 * Uploads `files` as documents of an account or a transaction (`POST /api/{accounts|transactions}/{id}/documents`). Plain
 * `fetch`: the generated client JSON-encodes the body, which drops the multipart files.
 */
export async function uploadDocuments(target: 'accounts' | 'transactions', id: string, files: File[]): Promise<void> {
  const formData = new FormData();
  files.forEach((file) => formData.append('file', file));
  const response = await fetch(`${apiBaseUrl}/api/${target}/${encodeURIComponent(id)}/documents`, { method: 'POST', body: formData });
  if (!response.ok) throw await responseError(response);
}

/**
 * Runs a query through `POST /api/query/csv` and returns the CSV file. A query error is thrown as an `ApiError`
 * carrying the same `{message, line, column}` body as `POST /api/query`.
 *
 * This is a plain `fetch` call: the generated fetcher would `JSON.parse` a CSV body that happens to be valid JSON,
 * and the download needs the raw blob.
 */
export async function exportQueryCsv(query: string): Promise<{ blob: Blob; filename: string }> {
  const response = await fetch(`${apiBaseUrl}/api/query/csv`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query }),
  });
  if (!response.ok) {
    reportUnauthorized(response.url, response.status);
    throw toApiError(response, await readBody(response));
  }
  const blob = await response.blob();
  return { blob, filename: filenameOf(response.headers.get('Content-Disposition')) ?? 'query.csv' };
}

async function readBody(response: Response): Promise<unknown> {
  const text = await response.text();
  try {
    return text ? JSON.parse(text) : undefined;
  } catch {
    return text;
  }
}

function toApiError(response: Response, data: unknown): ApiError {
  return new ApiError({ headers: response.headers, url: response.url, status: response.status, statusText: response.statusText, data });
}

function filenameOf(contentDisposition: string | null): string | null {
  if (!contentDisposition) return null;
  const encoded = /filename\*\s*=\s*(?:UTF-8'')?([^;]+)/i.exec(contentDisposition);
  if (encoded) {
    try {
      return decodeURIComponent(encoded[1].trim().replace(/^"|"$/g, ''));
    } catch {
      // fall back to the plain filename parameter
    }
  }
  const plain = /filename\s*=\s*("([^"]*)"|[^;]+)/i.exec(contentDisposition);
  return plain ? (plain[2] ?? plain[1]).trim() : null;
}
