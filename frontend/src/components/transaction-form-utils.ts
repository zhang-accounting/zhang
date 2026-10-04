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
  /** Editable metadata rows. */
  metas: MetaEntry[];
  /** The posting's `document` entries: not shown in the editor, sent back unchanged. */
  documents: MetaEntry[];
}

/** Request body of "create transaction"; "update transaction" takes the same body. */
type TransactionRequest = operations['create_new_transaction']['requestBody']['content']['application/json'];

/** A posting as sent to the create / update transaction API (the form always sends its `metas`). */
export type PostingRequest = TransactionRequest['postings'][number] & { metas: MetaEntry[] };

/** Request body shared by "create" and "update" transaction, as the form builds it. */
export type TransactionFormValue = Omit<TransactionRequest, 'narration' | 'postings'> & { narration: string; postings: PostingRequest[] };

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

/** How the ledger file is written: zhang (`.zhang`) or beancount (`.bean`, date only plus a `time` metadata, beancount quoting). */
export type LedgerFormat = 'zhang' | 'beancount';

/**
 * The format of a ledger from `/api/files`: the main file is listed first. Like the server (`is_beancount_endpoint`), a
 * main file ending in `.bc`, `.bean` or `.beancount` is a beancount ledger.
 */
export function ledgerFormat(files: (string | null | undefined)[] | undefined): LedgerFormat {
  return /\.(bc|bean|beancount)$/.test(files?.[0] ?? '') ? 'beancount' : 'zhang';
}

/** Characters the server writes as `\uXXXX` besides controls: invisible or text-reordering ones (`is_hidden_format_char`). */
const HIDDEN_FORMAT_CHAR = /[\u061c\u200b\u200e\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069\ufeff]/;

const ESCAPES: Record<string, string> = { '"': '\\"', '\\': '\\\\', '\n': '\\n', '\t': '\\t', '\r': '\\r' };
const BEANCOUNT_ESCAPES: Record<string, string> = { ...ESCAPES, '\u0008': '\\b', '\u000c': '\\f' };

/**
 * Mirrors the server's `quote_as(text, style)`: zhang writes every other control and hidden format character as `\uXXXX`;
 * beancount only knows `\b` and `\f` besides, and writes the rest raw.
 */
export function quote(text: string, format: LedgerFormat = 'zhang') {
  const escapes = format === 'beancount' ? BEANCOUNT_ESCAPES : ESCAPES;
  let output = '"';
  for (const char of text) {
    const escape = escapes[char];
    if (escape) output += escape;
    else if (format === 'zhang' && (/\p{Cc}/u.test(char) || HIDDEN_FORMAT_CHAR.test(char))) output += `\\u${char.charCodeAt(0).toString(16).padStart(4, '0')}`;
    else output += char;
  }
  return `${output}"`;
}

/**
 * A metadata key as the server writes it: bare when it reads back that way (no `"`, `:`, `(`, `)`, `,`, space, tab or line
 * break, not starting like a comment), quoted otherwise (`meta_key` in the exporter).
 */
export function metaKeyText(key: string, format: LedgerFormat = 'zhang') {
  const bare = key !== '' && !/[":(), \t\n\r]/.test(key) && !/^(\/\/|;|\*|#)/.test(key);
  return bare ? key : quote(key, format);
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
      { id: 0, account: undefined, amount: '', metas: [], documents: [] },
      { id: 1, account: undefined, amount: '', metas: [], documents: [] },
    ];
  }
  return postings.map((posting, idx) => ({
    id: idx,
    account: posting.account ?? undefined,
    amount: `${posting.unit?.number ?? ''} ${posting.unit?.commodity ?? ''}`.trim(),
    metas: posting.metas.filter((meta) => meta.key !== DOCUMENT_KEY).map((meta) => ({ key: meta.key, value: meta.value })),
    documents: posting.metas.filter((meta) => meta.key === DOCUMENT_KEY),
  }));
}

/** The request posting of a form row; `amount` is the row's parsed amount (anything but `ok` leaves the unit to the server). */
export function toPostingRequest(draft: PostingDraft, amount: AmountState): PostingRequest {
  return {
    account: draft.account ?? '',
    unit: amount.status === 'ok' ? { number: amount.number, commodity: amount.commodity } : null,
    metas: [...toRequestMetas(draft.metas), ...draft.documents],
  };
}

function byKey(a: MetaEntry, b: MetaEntry) {
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

export interface DirectiveTextOptions {
  /** `yyyy-MM-dd HH:mm:ss` in the ledger timezone. */
  datetime: string;
  /** Defaults to `zhang`. */
  format?: LedgerFormat;
  /** Parsed amount of each posting (same order as `value.postings`). */
  amounts: AmountState[];
  /** Shown instead of an invalid amount / a missing account. */
  invalidAmount: string;
  accountPlaceholder: string;
}

/**
 * The directive the server will write, in its exporter's layout: the header, the transaction metadata (sorted by key, 2
 * spaces), then each posting followed by its own metadata (sorted by key, 4 spaces). Transaction metadata must come before the
 * postings: a metadata line after a posting belongs to that posting.
 */
export function directiveText(value: TransactionFormValue, options: DirectiveTextOptions): string {
  const format = options.format ?? 'zhang';
  // beancount has no time of day: the server writes the date and keeps the time as a `time` metadata of the transaction
  const [date, time] = format === 'beancount' ? options.datetime.split(' ') : [options.datetime];
  const transactionMetas = time ? [...value.metas, { key: 'time', value: time }] : value.metas;
  const header = [
    date,
    value.flag || '*',
    quote(value.payee, format),
    quote(value.narration, format),
    ...value.tags.map((tag) => `#${tag}`),
    ...value.links.map((link) => `^${link}`),
  ].join(' ');
  const metaLine = (indent: string) => (meta: MetaEntry) => `${indent}${metaKeyText(meta.key, format)}: ${quote(meta.value, format)}`;
  const metaLines = [...transactionMetas].sort(byKey).map(metaLine('  '));
  const postingLines = value.postings.flatMap((posting, idx) => {
    const amount = options.amounts[idx];
    const unit = amount?.status === 'ok' ? `${amount.number} ${amount.commodity}` : !amount || amount.status === 'empty' ? '' : options.invalidAmount;
    const line = `  ${posting.account || options.accountPlaceholder} ${unit}`.trimEnd();
    return [line, ...[...posting.metas].sort(byKey).map(metaLine('    '))];
  });
  return [header, ...metaLines, ...postingLines].join('\n');
}
