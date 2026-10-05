// Pure helpers of `TransactionEditForm`. Type-only imports keep this module runnable by `node --test`
// (transaction-form-utils.test.ts).
import type { operations } from '@/api/schemas';
import type { JournalTransactionItem, MetaEntry } from '@/api/types';

/** The `document` metadata key: attached files, managed by uploads rather than edited as metadata. */
export const DOCUMENT_KEY = 'document';

/** One posting row of the form; `id` is a stable React key (rows can be removed from the middle). */
export interface PostingDraft {
  id: number;
  account: string | undefined;
  amount: string;
  /** The cost as the ledger writes it, e.g. `{150 USD}`, `{{1500 USD}}` or `{}`; empty for none. */
  cost: string;
  /** The price as the ledger writes it, e.g. `@ 6 USD` or `@@ 60 USD`; empty for none. */
  price: string;
  /** The comment at the end of the posting line, without the `;`; empty for none. */
  comment: string;
  /** Editable metadata rows. */
  metas: MetaEntry[];
  /** The posting's `document` entries: not shown in the editor, sent back unchanged. */
  documents: MetaEntry[];
}

/** An empty posting row. */
export function emptyDraft(id: number): PostingDraft {
  return { id, account: undefined, amount: '', cost: '', price: '', comment: '', metas: [], documents: [] };
}

/** Request body of "create transaction"; "update transaction" takes the same body. */
type TransactionRequest = operations['create_new_transaction']['requestBody']['content']['application/json'];

/** A posting as sent to the create / update transaction API (the form always sends its `metas`). */
export type PostingRequest = TransactionRequest['postings'][number] & { metas: MetaEntry[] };

/** Request body shared by "create" and "update" transaction, as the form builds it. */
export type TransactionFormValue = Omit<TransactionRequest, 'narration' | 'postings'> & { narration: string; postings: PostingRequest[] };

/**
 * Metadata rows as submitted: keys are trimmed, rows without a key are dropped (an empty editor row is not an entry), values
 * are kept verbatim. Key rules (e.g. a lowercase first letter on beancount ledgers) are left to the server, which answers 400.
 */
export function toRequestMetas(metas: MetaEntry[]): MetaEntry[] {
  return metas.filter((meta) => meta.key.trim() !== '').map((meta) => ({ key: meta.key.trim(), value: meta.value }));
}

/**
 * Form rows of an existing transaction (or two empty rows for a new one), keeping each posting's metadata and its cost, price
 * and comment as the journal shows them written (`written`, in the ledger's own syntax).
 */
export function toPostingDrafts(postings: JournalTransactionItem['postings'] | undefined): PostingDraft[] {
  if (!postings) return [emptyDraft(0), emptyDraft(1)];
  return postings.map((posting, idx) => ({
    id: idx,
    account: posting.account ?? undefined,
    amount: `${posting.unit?.number ?? ''} ${posting.unit?.commodity ?? ''}`.trim(),
    cost: posting.written?.cost ?? '',
    price: posting.written?.price ?? '',
    comment: posting.written?.comment ?? '',
    metas: posting.metas.filter((meta) => meta.key !== DOCUMENT_KEY).map((meta) => ({ key: meta.key, value: meta.value })),
    documents: posting.metas.filter((meta) => meta.key === DOCUMENT_KEY),
  }));
}

/** A field of a posting as sent: its text, trimmed, or `null` for an empty one: no units, or no cost, price or comment. */
function fieldText(text: string): string | null {
  const trimmed = text.trim();
  return trimmed === '' ? null : trimmed;
}

/**
 * The request posting of a form row. The amount is sent as typed, trimmed, for the server to read with the ledger's grammar
 * (`null` when empty: the posting booking completes); a number alone is in the operating currency. The cost, price and comment
 * are always sent, `null` when empty: the form shows the posting's own, so an empty field means none.
 */
export function toPostingRequest(draft: PostingDraft): PostingRequest {
  return {
    account: draft.account ?? '',
    unit: fieldText(draft.amount),
    cost: fieldText(draft.cost),
    price: fieldText(draft.price),
    comment: fieldText(draft.comment),
    metas: [...toRequestMetas(draft.metas), ...draft.documents],
  };
}
