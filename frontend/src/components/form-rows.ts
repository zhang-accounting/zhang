// The rows of the forms' built-in queries as the forms read them: pure, so `node --test` checks them against what the
// endpoints they replace answered (form-rows.test.ts); its imports are type-only or carry their extension for that.
import type { Builtins } from '@/api/builtins';
import type { LedgerDateTime } from './ledger-datetime';
import { type LedgerInstant, datetimeOf } from './ledger-now.ts';

export type PayeeRow = Builtins['journals.payees']['row'];
export type AccountRow = Builtins['journals.accounts']['row'] | Builtins['accounts.opened']['row'];

/** What the new-transaction form suggests. */
export interface NewTransactionInfo {
  /** the ledger's current wall-clock time, the default date and time of a new transaction */
  now: LedgerDateTime;
  /** every payee of the ledger's transactions, sorted */
  payee: string[];
  /** the accounts open at the transaction's date and time, by the rule the ledger checks the transaction with */
  account_name: string[];
}

/** The payees of the rows of `journals.payees`, in their (sorted) order. */
export function payeeNames(rows: PayeeRow[]): string[] {
  return rows.flatMap((row) => (row.payee ? [row.payee] : []));
}

/** The account names of the rows of `journals.accounts` or `accounts.opened`, in their (sorted) order. */
export function accountNames(rows: AccountRow[]): string[] {
  return rows.flatMap((row) => (row.account ? [row.account] : []));
}

/** The form's suggestions from the ledger's instant and the rows of `journals.payees` and `journals.accounts`. */
export function newTransactionInfo(now: LedgerInstant, payees: PayeeRow[], accounts: AccountRow[]): NewTransactionInfo {
  return { now: datetimeOf(now), payee: payeeNames(payees), account_name: accountNames(accounts) };
}
