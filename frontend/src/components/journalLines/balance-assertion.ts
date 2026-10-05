// The balance assertion of a journal row, in the one shape both journals give it: `GET /api/journals` (a `BalanceCheck` item)
// and `GET /api/accounts/{account}/journals` (an assertion row). Type-only imports keep this module runnable by `node --test`
// (balance-assertion.test.ts).
import type { JournalBalanceCheckItem } from '@/api/types';
import type { operations } from '@/api/schemas';

type AccountJournalRow = operations['get_account_journals']['responses']['200']['content']['application/json']['data'][number];

/** What a balance assertion asserted and what the ledger found, the same fields on both journals. */
export type BalanceAssertion = Pick<JournalBalanceCheckItem, 'asserted' | 'checked_balance' | 'difference' | 'tolerance' | 'passed'>;

/**
 * The assertion of a journal row: that of a `BalanceCheck` item of the journal, or of an assertion row of an account's journal;
 * `null` for a posting of an account's journal.
 */
export function assertionOf(row: JournalBalanceCheckItem | AccountJournalRow): BalanceAssertion | null {
  const { asserted, checked_balance, difference, tolerance, passed } = row;
  if (!asserted || !checked_balance || !difference || passed === null || passed === undefined) return null;
  return { asserted, checked_balance, difference, tolerance: tolerance ?? null, passed };
}
