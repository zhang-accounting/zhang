/**
 * Where a ledger error is, for people: the file and the lines of its directive (#493). The server records a
 * directive's byte offsets, which writers use to replace it, and the line where it starts; its last line follows from
 * its text.
 */

/** What the location needs of an error's span; `line` is `null` or absent when the server does not know it. */
export interface SpanPosition {
  filename?: string | null;
  content: string;
  line?: number | null;
}

/** The first and last line of the directive, or `null` when the server does not know where it starts (a span not read from a file). */
export function spanLines(span: SpanPosition | null | undefined): { start: number; end: number } | null {
  if (span?.line == null) return null;
  const lines = span.content.trimEnd().split('\n').length;
  return { start: span.line, end: span.line + lines - 1 };
}

/** The location text: `main.zhang · lines 5–7`, `main.zhang · line 5`, or only the file when its lines are unknown. */
export function errorLocation(span: SpanPosition | null | undefined, t: (key: string, params: Record<string, string | number>) => string): string {
  if (!span) return '';
  const file = span.filename ?? '';
  const lines = spanLines(span);
  if (!lines) return file;
  return lines.start === lines.end
    ? t('ERROR_BOX_LOCATION_LINE', { file, line: lines.start })
    : t('ERROR_BOX_LOCATION', { file, start: lines.start, end: lines.end });
}
