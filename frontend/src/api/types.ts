import { OpReturnType } from 'openapi-typescript-fetch';
import { operations } from './schemas';

export type JournalItem = OpReturnType<operations['get_journals']>['data']['records'][number]
export type JournalTransactionItem = Extract<JournalItem, {type: 'Transaction'}>
export type JournalBalanceCheckItem = Extract<JournalItem, {type: 'BalanceCheck'}>
export type JournalBalancePadItem = Extract<JournalItem, {type: 'BalancePad'}>

export type Account = OpReturnType<operations['get_account_info']>['data']
export type AccountListItem = OpReturnType<operations['get_account_list']>['data'][number]

export type LedgerError = OpReturnType<operations['get_errors']>['data']['records'][number]
export type Document = OpReturnType<operations['get_documents']>['data'][number]

export type BudgetListItem = OpReturnType<operations['get_budget_list']>['data'][number]
export enum AccountType {
  Income = "Income",
  Expenses = "Expenses",
  Assets = "Assets",
  Liabilities = "Liabilities",
  Equity = "Equity"
}
