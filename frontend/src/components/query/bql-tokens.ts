// The tokens the query editor colours. Runnable by `node --test` (bql-tokens.test.ts).

/**
 * The pattern of the tokens of a query, one capture group per kind, in this order: string, date, number, `#table`, keyword,
 * function name. The keywords are the parser's, as `/api/query/schema` lists them; with none (while it loads) no word is
 * one.
 */
export function tokenRegexp(keywords: readonly string[]): RegExp {
  const words = keywords.filter((word) => /^[a-z_]+$/i.test(word));
  return new RegExp(
    [
      /("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?)/.source,
      /\b(\d{4}-\d{2}-\d{2})\b/.source,
      /\b(\d+(?:\.\d+)?)\b/.source,
      /(?<![\w#])(#[a-z_][a-z0-9_]*)/.source,
      words.length > 0 ? `\\b(${words.join('|')})\\b` : '(?!)()',
      /\b([a-z_][a-z0-9_]*)(?=\s*\()/.source,
    ].join('|'),
    'gi',
  );
}

/** The kind of the token a match of [`tokenRegexp`] found. */
export function tokenKind(match: RegExpExecArray): 'string' | 'date' | 'number' | 'table' | 'keyword' | 'function' {
  if (match[1] !== undefined) return 'string';
  if (match[2] !== undefined) return 'date';
  if (match[3] !== undefined) return 'number';
  if (match[4] !== undefined) return 'table';
  if (match[5] !== undefined) return 'keyword';
  return 'function';
}
