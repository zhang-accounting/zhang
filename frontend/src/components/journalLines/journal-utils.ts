// Type-only imports keep this module runnable by `node --test` (journal-utils.test.ts).
import type { JournalBalanceCheckItem, JournalItem, JournalTransactionItem, MetaEntry } from '@/api/types';
import { DOCUMENT_KEY } from '../transaction-form-utils.ts';

/** `true` when the balance assertion held, within its tolerance. The server decides it: a check within its tolerance passes. */
export function isBalanceCheckPassed(data: Pick<JournalBalanceCheckItem, 'passed'>) {
  return data.passed;
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

/**
 * The documents of a transaction: its own `document` metadata, then that of its postings, each path once. The server links a
 * posting's document to the transaction too: older zhang appended uploads after the postings, which a beancount ledger reads
 * as metadata of the last posting, and users may write `document:` under a posting by hand.
 */
export function transactionDocuments(data: { metas: MetaEntry[]; postings: { metas: MetaEntry[] }[] }): MetaEntry[] {
  const seen = new Set<string>();
  return [data.metas, ...data.postings.map((posting) => posting.metas)].flat().filter((meta) => {
    if (meta.key !== DOCUMENT_KEY || seen.has(meta.value)) return false;
    seen.add(meta.value);
    return true;
  });
}

export function hasDocuments(data: JournalTransactionItem) {
  return transactionDocuments(data).length > 0;
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

/** The journal page size the Journals page asks for. */
export const JOURNAL_PAGE_SIZE = 100;

/**
 * The parameters of the built-in query `journals.page` for a page of the journal, as the server binds them: an empty
 * keyword, and no tags or links, filter nothing.
 */
export function journalQueryParams(page: number, keyword: string, tags: string[], links: string[]) {
  return {
    keyword: keyword === '' ? null : keyword,
    tags: tags.length > 0 ? tags : null,
    links: links.length > 0 ? links : null,
    size: JOURNAL_PAGE_SIZE,
    offset: (Math.max(page, 1) - 1) * JOURNAL_PAGE_SIZE,
  };
}
