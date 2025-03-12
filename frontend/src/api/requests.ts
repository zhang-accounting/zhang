import { openAPIFetcher } from './fetcher';

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

export const uploadTransactionDocument = openAPIFetcher.path('/api/transactions/{transaction_id}/documents').method('post').create();

export const uploadAccountDocument = openAPIFetcher.path('/api/accounts/{account_name}/documents').method('post').create();

export const createAccountBalance = openAPIFetcher.path('/api/accounts/{account_name}/balances').method('post').create();
