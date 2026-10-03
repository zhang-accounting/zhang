import { FetchReturnType, OpReturnType } from 'openapi-typescript-fetch';
import type { retrieveQuerySchema, retrieveSavedQueries } from './requests';
import { operations } from './schemas';

/** `{ key, value }` metadata entry of a transaction or a posting. */
export type MetaEntry = { key: string; value: string };

// TEMPORARY(pm): posting-level `metas` is hand-written until `schemas.ts` is regenerated from a server that has posting
// metadata. Then delete `WithPostingMetas` and use the generated record type directly (`metas` becomes required).
type GeneratedJournalItem = OpReturnType<operations['get_journals']>['data']['records'][number];
type WithPostingMetas<T> = T extends { postings: (infer P)[] } ? Omit<T, 'postings'> & { postings: (P & { metas?: MetaEntry[] })[] } : T;

export type JournalItem = WithPostingMetas<GeneratedJournalItem>;
export type JournalTransactionItem = Extract<JournalItem, { type: 'Transaction' }>;
export type JournalPosting = JournalTransactionItem['postings'][number];
export type JournalBalanceCheckItem = Extract<JournalItem, { type: 'BalanceCheck' }>;
export type JournalBalancePadItem = Extract<JournalItem, { type: 'BalancePad' }>;

export type Account = OpReturnType<operations['get_account_info']>['data'];
export type AccountListItem = OpReturnType<operations['get_account_list']>['data'][number];

export type LedgerError = OpReturnType<operations['get_errors']>['data']['records'][number];
export type Document = OpReturnType<operations['get_documents']>['data'][number];

export type BudgetListItem = OpReturnType<operations['get_budget_list']>['data'][number];
export enum AccountType {
  Income = 'Income',
  Expenses = 'Expenses',
  Assets = 'Assets',
  Liabilities = 'Liabilities',
  Equity = 'Equity',
}

// Query explore (`POST /api/query`). Cell shapes are kept hand-written instead of being derived from the
// generated schema, so the typed renderer does not depend on how the generator models the untyped cells.
export interface QueryColumn {
  name: string;
  type: string;
}
export interface QueryResult {
  columns: QueryColumn[];
  rows: unknown[][];
}
export interface QueryAmount {
  number: string;
  currency: string;
}
export interface QueryCost {
  number: string;
  currency: string;
  date: string | null;
  label: string | null;
}
export interface QueryPosition {
  units: QueryAmount;
  cost: QueryCost | null;
}
export interface QueryInventory {
  positions: QueryPosition[];
}
export interface QueryError {
  message: string;
  line: number | null;
  column: number | null;
}
export type QuerySchema = FetchReturnType<typeof retrieveQuerySchema>['data'];
export type QueryTableDoc = QuerySchema['tables'][number];
export type QueryTableColumnDoc = QueryTableDoc['columns'][number];
export type SavedQuery = FetchReturnType<typeof retrieveSavedQueries>['data'][number];
