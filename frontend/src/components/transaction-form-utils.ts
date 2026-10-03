// Pure helpers of `TransactionEditForm`. Type-only imports keep this module runnable by `node --test`
// (transaction-form-utils.test.ts).
import type { JournalTransactionItem, MetaEntry } from '@/api/types';

/** One posting row of the form; `id` is a stable React key (rows can be removed from the middle). */
export interface PostingDraft {
  id: number;
  account: string | undefined;
  amount: string;
  metas: MetaEntry[];
}

/** A posting as sent to the create / update transaction API. */
export interface PostingRequest {
  account: string;
  unit: { number: string; commodity: string } | null;
  // TEMPORARY(pm): posting `metas` is not in the generated request schema yet; derive this type from `operations` once
  // `schemas.ts` is regenerated from a server with posting metadata.
  metas: MetaEntry[];
}

/** Request body shared by "create" and "update" transaction. */
export interface TransactionFormValue {
  datetime: string;
  payee: string;
  narration: string;
  flag?: string | null;
  postings: PostingRequest[];
  tags: string[];
  links: string[];
  metas: MetaEntry[];
}

export type AmountState = { status: 'empty' } | { status: 'ok'; number: string; commodity: string } | { status: 'cost_price' | 'no_commodity' | 'invalid' };

/** `<number> <COMMODITY>` (the ledger's `commodity_name` grammar); the commodity may be left out when there is a fallback. */
const AMOUNT_PATTERN = /^(-?\d+(?:\.\d+)?)(?:\s*([A-Za-z][A-Za-z0-9._'-]*))?$/;

/**
 * `"-21.5 CNY"` → `{ number: '-21.5', commodity: 'CNY' }`; a bare number falls back to the operating currency. Anything else
 * (cost `{…}`, price `@ …`, expressions, extra tokens) is rejected: the API only stores `{ number, commodity }` per posting.
 */
export function parseAmount(raw: string, fallbackCommodity?: string): AmountState {
  const text = raw.trim();
  if (text === '') return { status: 'empty' };
  if (/[{}@]/.test(text)) return { status: 'cost_price' };
  const match = AMOUNT_PATTERN.exec(text);
  if (!match) return { status: 'invalid' };
  const commodity = match[2] ?? fallbackCommodity;
  if (!commodity) return { status: 'no_commodity' };
  return { status: 'ok', number: match[1], commodity };
}

/** Mirrors the server's `escape_with_quote`. */
export function quote(text: string) {
  const escaped = text.replace(/["\\$`]/g, (char) => `\\${char}`).replace(/[\n\r\t]/g, (char) => ({ '\n': '\\n', '\r': '\\r', '\t': '\\t' })[char] ?? char);
  return `"${escaped}"`;
}

/**
 * Metadata rows as submitted: keys are trimmed, rows without a key are dropped (an empty editor row is not an entry), values
 * are kept verbatim. Key rules (e.g. a lowercase first letter on beancount ledgers) are left to the server, which answers 400.
 */
export function toRequestMetas(metas: MetaEntry[]): MetaEntry[] {
  return metas.filter((meta) => meta.key.trim() !== '').map((meta) => ({ key: meta.key.trim(), value: meta.value }));
}

/** Form rows of an existing transaction (or two empty rows for a new one), keeping each posting's metadata. */
export function toPostingDrafts(postings: JournalTransactionItem['postings'] | undefined): PostingDraft[] {
  if (!postings) {
    return [
      { id: 0, account: undefined, amount: '', metas: [] },
      { id: 1, account: undefined, amount: '', metas: [] },
    ];
  }
  return postings.map((posting, idx) => ({
    id: idx,
    account: posting.account ?? undefined,
    amount: `${posting.unit?.number ?? ''} ${posting.unit?.commodity ?? ''}`.trim(),
    metas: (posting.metas ?? []).map((meta) => ({ key: meta.key, value: meta.value })),
  }));
}

/** The request posting of a form row; `amount` is the row's parsed amount (anything but `ok` leaves the unit to the server). */
export function toPostingRequest(draft: PostingDraft, amount: AmountState): PostingRequest {
  return {
    account: draft.account ?? '',
    unit: amount.status === 'ok' ? { number: amount.number, commodity: amount.commodity } : null,
    metas: toRequestMetas(draft.metas),
  };
}

function byKey(a: MetaEntry, b: MetaEntry) {
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

export interface DirectiveTextOptions {
  /** `yyyy-MM-dd HH:mm:ss` in the ledger timezone. */
  datetime: string;
  /** Parsed amount of each posting (same order as `value.postings`). */
  amounts: AmountState[];
  /** Shown instead of an invalid amount / a missing account. */
  invalidAmount: string;
  accountPlaceholder: string;
}

/**
 * The directive the server will write, in its exporter's layout: the header, the transaction metadata (sorted by key), then
 * each posting followed by its own metadata (sorted, indented one level deeper). Transaction metadata must come before the
 * postings: a metadata line after a posting belongs to that posting.
 */
export function directiveText(value: TransactionFormValue, options: DirectiveTextOptions): string {
  const header = [
    options.datetime,
    value.flag || '*',
    quote(value.payee),
    quote(value.narration),
    ...value.tags.map((tag) => `#${tag}`),
    ...value.links.map((link) => `^${link}`),
  ].join(' ');
  const metaLines = [...value.metas].sort(byKey).map((meta) => `  ${meta.key}: ${quote(meta.value)}`);
  const postingLines = value.postings.flatMap((posting, idx) => {
    const amount = options.amounts[idx];
    const unit = amount?.status === 'ok' ? `${amount.number} ${amount.commodity}` : !amount || amount.status === 'empty' ? '' : options.invalidAmount;
    const line = `  ${posting.account || options.accountPlaceholder} ${unit}`.trimEnd();
    return [line, ...[...posting.metas].sort(byKey).map((meta) => `    ${meta.key}: ${quote(meta.value)}`)];
  });
  return [header, ...metaLines, ...postingLines].join('\n');
}
