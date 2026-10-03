import BigNumber from 'bignumber.js';
import { JournalBalanceCheckItem, JournalItem, JournalTransactionItem } from '@/api/types';

/** `true` when the balance assertion matches the accumulated amount. */
export function isBalanceCheckPassed(data: JournalBalanceCheckItem) {
  const posting = data.postings[0];
  return new BigNumber(posting.account_after.number).eq(new BigNumber(posting.account_before.number));
}

/** Problem level of a journal: unbalanced transaction / failed check → `error`, `!` flag → `warning`. */
export function journalStatus(data: JournalItem): 'ok' | 'warning' | 'error' {
  if (data.type === 'Transaction') {
    if (!data.is_balanced) return 'error';
    if (data.flag === '!') return 'warning';
  }
  if (data.type === 'BalanceCheck' && !isBalanceCheckPassed(data)) return 'error';
  return 'ok';
}

export function hasDocuments(data: JournalTransactionItem) {
  return data.metas.some((meta) => meta.key === 'document');
}

/**
 * Why the transaction form cannot safely rewrite this transaction, or `null` when it can.
 *
 * The update API rebuilds every posting from `{ account, unit, metas }` only, so cost (`{…}`) and price (`@ …`) annotations would be
 * silently dropped. The journal payload exposes `cost`, but not prices: a balanced transaction can only mix commodities through
 * a cost or a price, so postings in more than one commodity are treated as "has cost / price" too. Posting comments and posting
 * flags are not in the payload at all (see `TransactionEditModal`, which asks for confirmation instead).
 */
export function transactionEditBlocker(data: JournalTransactionItem): 'cost_or_price' | null {
  if (data.postings.some((posting) => posting.cost)) return 'cost_or_price';
  const commodities = new Set(data.postings.map((posting) => posting.unit?.commodity ?? posting.inferred_unit.commodity));
  return commodities.size > 1 ? 'cost_or_price' : null;
}

/** Raw Edit deep link. Journals do not expose their source file, so this opens the editor on its default file. */
export const RAW_EDIT_URI = '/edit';
