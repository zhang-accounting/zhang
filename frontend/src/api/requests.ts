import { ApiError } from 'openapi-typescript-fetch';
import { apiBaseUrl, openAPIFetcher, reportUnauthorized } from './fetcher';

export const retrieveJournals = openAPIFetcher.path('/api/journals').method('get').create();

export const retrieveBudgets = openAPIFetcher.path('/api/budgets').method('get').create();

export const retrieveDocuments = openAPIFetcher.path('/api/documents').method('get').create();

export const retrieveStatisticGraph = openAPIFetcher.path('/api/statistic/graph').method('get').create();

export const retrieveFiles = openAPIFetcher.path('/api/files').method('get').create();

export const retrieveStatisticSummary = openAPIFetcher.path('/api/statistic/summary').method('get').create();

export const retrieveStatisticByAccountType = openAPIFetcher.path('/api/statistic/{account_type}').method('get').create();

export const retrieveOptions = openAPIFetcher.path('/api/options').method('get').create();

export const retrievePlugins = openAPIFetcher.path('/api/plugins').method('get').create();

export const retrieveAccountInfo = openAPIFetcher.path('/api/accounts/{account_name}').method('get').create();

export const retrieveAccountBalance = openAPIFetcher.path('/api/accounts/{account_name}/balances').method('get').create();

export const retrieveAccountJournals = openAPIFetcher.path('/api/accounts/{account_name}/journals').method('get').create();

export const retrieveAccountDocuments = openAPIFetcher.path('/api/accounts/{account_name}/documents').method('get').create();

export const retrieveBudgetInfo = openAPIFetcher.path('/api/budgets/{budget_name}').method('get').create();

export const retrieveBudgetEvent = openAPIFetcher.path('/api/budgets/{budget_name}/interval/{year}/{month}').method('get').create();

export const retrieveCommodityInfo = openAPIFetcher.path('/api/commodities/{commodity_name}').method('get').create();

export const retrieveNewTransactionInfo = openAPIFetcher.path('/api/for-new-transaction').method('get').create();

export const retrieveFile = openAPIFetcher.path('/api/files/{file_path}').method('get').create();

export const updateFile = openAPIFetcher.path('/api/files/{file_path}').method('put').create();

export const createNewTransaction = openAPIFetcher.path('/api/transactions').method('post').create();

export const createBatchBalance = openAPIFetcher.path('/api/accounts/batch-balances').method('post').create();

export const reloadLedger = openAPIFetcher.path('/api/reload').method('post').create();

export const updateTransaction = openAPIFetcher.path('/api/transactions/{transaction_id}').method('put').create();

export const createAccountBalance = openAPIFetcher.path('/api/accounts/{account_name}/balances').method('post').create();

export const executeQuery = openAPIFetcher.path('/api/query').method('post').create();

export const retrieveQuerySchema = openAPIFetcher.path('/api/query/schema').method('get').create();

export const retrieveSavedQueries = openAPIFetcher.path('/api/query/saved').method('get').create();

export const retrieveBuiltinQueryText = openAPIFetcher.path('/api/query/builtins/{name}/text').method('post').create();

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
