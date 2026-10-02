import { FetchReturnType, OpReturnType } from 'openapi-typescript-fetch';
import type { retrieveQuerySchema } from './requests';
import { operations } from './schemas';

export type JournalItem = OpReturnType<operations['get_journals']>['data']['records'][number];
export type JournalTransactionItem = Extract<JournalItem, { type: 'Transaction' }>;
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
