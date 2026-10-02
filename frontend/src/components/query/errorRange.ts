import { Text } from '@uiw/react-codemirror';

export interface ErrorRange {
  lineFrom: number;
  from: number;
  to: number;
}

/**
 * Converts a 1-based line/column (column counted in characters) into an editor range covering the token at that position.
 */
export function errorRangeOf(doc: Text, line: number | null, column: number | null): ErrorRange | null {
  if (line === null || line < 1 || line > doc.lines) return null;
  const docLine = doc.line(line);
  if (column === null || column < 1) {
    return { lineFrom: docLine.from, from: docLine.from, to: docLine.from };
  }
  const offset = Array.from(docLine.text)
    .slice(0, column - 1)
    .join('').length;
  const from = docLine.from + Math.min(offset, docLine.length);
  const token = /^(\w+|\S)/.exec(docLine.text.slice(from - docLine.from));
  return { lineFrom: docLine.from, from, to: from + (token ? token[0].length : 0) };
}
