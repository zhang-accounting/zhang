// The error box's items and pages from the rows of the built-in query `journals.errors`. Type-only imports keep this
// module runnable by `node --test` (ledger-errors.test.ts).
import type { Builtins } from '@/api/builtins';

export type LedgerErrorRow = Builtins['journals.errors']['row'];

/** Where the directive of an error is in its file: its byte range, which writers use to replace it, and where it starts. */
export interface LedgerErrorSpan {
  /** byte offset in the file where the directive starts */
  start: number;
  /** byte offset in the file just after the directive ends */
  end: number;
  /** the directive's text */
  content: string;
  filename: string | null;
  /** 1-based line in the file where the directive starts; `null` when unknown (a directive not read from a file) */
  line: number | null;
  /** 1-based column in its line where the directive starts, counting characters; `null` when unknown */
  column: number | null;
}

/** An error of the ledger as the error box shows it. */
export interface LedgerError {
  id: string;
  /** the directive the error is on; `null` for an error without one */
  span: LedgerErrorSpan | null;
  /** the kind of the error, the key of its title (`ERROR.<kind>`) */
  error_type: string;
  /** the error's details by name, which a title may show */
  metas: Record<string, string>;
}

/** One page of the ledger's errors, as the error box pages through them. */
export interface LedgerErrorPage {
  total_count: number;
  total_page: number;
  page_size: number;
  current_page: number;
  records: LedgerError[];
}

/** An error from a row of `journals.errors`: a span when the row knows where the directive starts. */
export function ledgerErrorOf(row: LedgerErrorRow): LedgerError {
  const span: LedgerErrorSpan | null =
    row.span_start != null
      ? {
          start: row.span_start,
          end: row.span_end ?? row.span_start,
          content: row.source ?? '',
          filename: row.file,
          line: row.line,
          column: row.column,
        }
      : null;
  // a key given twice keeps its last value
  const metas = Object.fromEntries((row.metas ?? []).map((meta) => [meta.key, meta.value]));
  return { id: row.id ?? '', span, error_type: row.kind ?? '', metas };
}

/**
 * Page `page` (from 1) of `size` errors: the rows of `journals.errors` run with `size` and `offset = (page - 1) * size`,
 * and `total`, the number of errors before the page's `LIMIT` and `OFFSET`.
 */
export function ledgerErrorPage(rows: LedgerErrorRow[], total: number, page: number, size: number): LedgerErrorPage {
  return {
    total_count: total,
    total_page: Math.floor(total / size) + (total % size === 0 ? 0 : 1),
    page_size: size,
    current_page: page,
    records: rows.map(ledgerErrorOf),
  };
}
