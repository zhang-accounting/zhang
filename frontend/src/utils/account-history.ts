// The balance history of an account's page from the rows of the built-in query `accounts.balance_history`. Type-only
// imports keep this module runnable by `node --test` (account-history.test.ts).
import type { Builtins } from '@/api/builtins';

export type BalanceHistoryRow = Builtins['accounts.balance_history']['row'];

/** The balance of the account and its sub-accounts at the end of a day, in one commodity. */
export interface BalancePoint {
  /** the ledger date, `YYYY-MM-DD` */
  date: string;
  balance: { number: string; commodity: string };
}

/**
 * The history per commodity, in the order of the rows (by date): the graph draws one commodity at a time. A `null`
 * balance is zero in the row's commodity.
 */
export function balanceHistoryByCommodity(rows: BalanceHistoryRow[]): Record<string, BalancePoint[]> {
  const history: Record<string, BalancePoint[]> = {};
  for (const row of rows) {
    const commodity = row.currency ?? '';
    (history[commodity] ??= []).push({
      date: row.date ?? '',
      balance: { number: row.balance?.number ?? '0', commodity: row.balance?.currency ?? commodity },
    });
  }
  return history;
}
