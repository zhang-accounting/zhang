// The server's preview of the transaction form (`POST /api/transactions/preview`, and
// `POST /api/transactions/{transaction_id}/preview` for an edit): what saving would write, the fields it would refuse, and what
// the ledger would report, found by the server's own parser, exporter and booking. Type-only imports keep this module runnable
// by `node --test` (transaction-preview.test.ts).
import type { operations } from '@/api/schemas';

/** What the server answers for a transaction request, without writing it. */
export type TransactionPreview = operations['preview_new_transaction']['responses']['200']['content']['application/json']['data'];

/** A field of a request the server would refuse with a 400. */
export type TransactionFieldError = TransactionPreview['field_errors'][number];

/** Why the server would refuse a field. */
export type FieldErrorKind = TransactionFieldError['kind'];

/** Every kind of field error, each with a message of the form in `ledger.txn.field_error.<kind>`. */
export const FIELD_ERROR_KINDS = [
  'invalid_account',
  'beancount_account',
  'invalid_commodity',
  'beancount_commodity',
  'invalid_amount',
  'invalid_cost',
  'invalid_price',
  'beancount_meta_key',
  'invalid_tag',
  'beancount_tag',
  'invalid_link',
  'beancount_link',
  'invalid_flag',
] as const satisfies readonly FieldErrorKind[];

// every kind the server sends is listed above
const allKinds: Exclude<FieldErrorKind, (typeof FIELD_ERROR_KINDS)[number]> extends never ? true : never = true;
void allKinds;

/** The `t` of i18next, as far as the form's messages need it. */
export type Translate = (key: string, options: { value: string; defaultValue: string }) => string;

/**
 * The message of a field error in the user's language: `ledger.txn.field_error.<kind>` with the value it is about, or the
 * server's own message for a kind this client does not know.
 */
export function fieldErrorText(error: TransactionFieldError, t: Translate): string {
  return t(`ledger.txn.field_error.${error.kind}`, { value: error.value, defaultValue: error.message });
}

/** An error the ledger would report against the transaction once written. */
export type TransactionLedgerError = TransactionPreview['errors'][number];

/** How long the form waits after the last change before it asks for a preview. */
export const PREVIEW_DELAY = 300;

/** The preview of one request, by its `key`: the server's answer, or the message of the request that failed. */
export interface PreviewState {
  key: string;
  preview?: TransactionPreview;
  error?: string;
}

/** The key of a request: two requests with the same key get the same preview. */
export function previewKey(request: unknown): string {
  return JSON.stringify(request);
}

/**
 * The errors of the fields of one posting (its index), or of the transaction (`null`), by field: the first error of each. A
 * preview for another request than `key` has none, as its fields may be fixed already.
 */
export function fieldErrors(
  state: PreviewState | undefined,
  key: string,
  posting: number | null,
): Partial<Record<TransactionFieldError['field'], TransactionFieldError>> {
  const errors: Partial<Record<TransactionFieldError['field'], TransactionFieldError>> = {};
  if (state?.key !== key) return errors;
  for (const error of state.preview?.field_errors ?? []) {
    if ((error.posting ?? null) === posting) errors[error.field] ??= error;
  }
  return errors;
}

/** Whether the server refuses the request `key` as it is: its preview names a field. Saving is pointless until it is fixed. */
export function refused(state: PreviewState | undefined, key: string): boolean {
  return state?.key === key && (state.preview?.field_errors.length ?? 0) > 0;
}

/** What the transaction is unbalanced by, as `0.50 CNY, 1 USD` at each commodity's precision; `null` when it balances. */
export function unbalancedText(preview: TransactionPreview | undefined): string | null {
  const unbalanced = preview?.unbalanced ?? [];
  return unbalanced.length > 0 ? unbalanced.map((amount) => `${amount.number} ${amount.commodity}`).join(', ') : null;
}

/** The ledger errors to list under the postings: all but `UnbalancedTransaction`, which `unbalancedText` tells with its amounts. */
export function ledgerErrors(preview: TransactionPreview | undefined): TransactionLedgerError[] {
  return (preview?.errors ?? []).filter((error) => error.error_type !== 'UnbalancedTransaction');
}

/**
 * Asks for the preview of the latest request only: `request` runs `fetch` `delay` ms after the last call, and `onAnswer` gets
 * each answer unless a newer request was answered first. `cancel` drops the request waiting and every answer still to come.
 */
export function createPreviewer(onAnswer: (state: PreviewState) => void, delay = PREVIEW_DELAY) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let sent = 0;
  let answered = 0;
  let cancelled = false;
  return {
    request(key: string, fetch: () => Promise<TransactionPreview>, describe: (error: unknown) => Promise<string>) {
      clearTimeout(timer);
      timer = setTimeout(async () => {
        const id = ++sent;
        let state: PreviewState;
        try {
          state = { key, preview: await fetch() };
        } catch (error) {
          state = { key, error: await describe(error) };
        }
        if (cancelled || id < answered) return;
        answered = id;
        onAnswer(state);
      }, delay);
    },
    cancel() {
      cancelled = true;
      clearTimeout(timer);
    },
  };
}
