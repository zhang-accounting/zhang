import { QueryError } from '@/api/types';
import { ErrorRange, errorRangeOf } from '@/components/query/errorRange';
import CodeMirror, {
  Decoration,
  DecorationSet,
  EditorView,
  keymap,
  MatchDecorator,
  Prec,
  StateEffect,
  StateField,
  ViewPlugin,
  ViewUpdate,
} from '@uiw/react-codemirror';
import { useTheme } from 'next-themes';
import { useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '@/lib/utils';

// Lightweight BQL highlighting built on the view package only, since no CodeMirror language package is installed.
const KEYWORDS =
  'select|distinct|from|where|group|by|order|asc|desc|limit|as|and|or|not|in|is|null|true|false|open|close|on|clear|balances|journal|at|pivot|having';
// one capture group per token kind, in this order: string, date, number, `#table`, keyword, function name
const TOKEN_REGEXP = new RegExp(
  [
    /("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?)/.source,
    /\b(\d{4}-\d{2}-\d{2})\b/.source,
    /\b(\d+(?:\.\d+)?)\b/.source,
    /(?<![\w#])(#[a-z_][a-z0-9_]*)/.source,
    `\\b(${KEYWORDS})\\b`,
    /\b([a-z_][a-z0-9_]*)(?=\s*\()/.source,
  ].join('|'),
  'gi',
);

const tokenMarks = {
  string: Decoration.mark({ class: 'cm-bql-string' }),
  date: Decoration.mark({ class: 'cm-bql-date' }),
  number: Decoration.mark({ class: 'cm-bql-number' }),
  table: Decoration.mark({ class: 'cm-bql-table' }),
  keyword: Decoration.mark({ class: 'cm-bql-keyword' }),
  function: Decoration.mark({ class: 'cm-bql-function' }),
};

const tokenMatcher = new MatchDecorator({
  regexp: TOKEN_REGEXP,
  decoration: (match) => {
    if (match[1] !== undefined) return tokenMarks.string;
    if (match[2] !== undefined) return tokenMarks.date;
    if (match[3] !== undefined) return tokenMarks.number;
    if (match[4] !== undefined) return tokenMarks.table;
    if (match[5] !== undefined) return tokenMarks.keyword;
    return tokenMarks.function;
  },
});

const bqlHighlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = tokenMatcher.createDeco(view);
    }

    update(update: ViewUpdate) {
      this.decorations = tokenMatcher.updateDeco(update, this.decorations);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

const setErrorRange = StateEffect.define<ErrorRange | null>();

const errorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decorations, tr) {
    let next = decorations.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setErrorRange)) {
        const range = effect.value;
        if (range === null) {
          next = Decoration.none;
        } else {
          const ranges = [Decoration.line({ class: 'cm-query-error-line' }).range(range.lineFrom)];
          if (range.to > range.from) {
            ranges.push(Decoration.mark({ class: 'cm-query-error' }).range(range.from, range.to));
          }
          next = Decoration.set(ranges, true);
        }
      }
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

/**
 * Syntax colours from the chart palette, pulled towards `--foreground` so they read as text (>= 5:1 on the card in light and
 * dark; amber needs the larger share). The tokens switch with `.dark`, so one rule covers both themes.
 */
const syntaxColor = (chart: number, share = 80) => `color-mix(in oklch, var(--chart-${chart}) ${share}%, var(--foreground))`;

const queryEditorTheme = EditorView.baseTheme({
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '&.cm-focused': { outline: 'none' },
  '.cm-bql-keyword': { color: syntaxColor(5), fontWeight: '600' },
  '.cm-bql-function': { color: syntaxColor(3) },
  '.cm-bql-table': { color: syntaxColor(3), fontWeight: '600' },
  '.cm-bql-string': { color: syntaxColor(1) },
  '.cm-bql-number': { color: syntaxColor(2) },
  '.cm-bql-date': { color: syntaxColor(4, 60) },
  '.cm-query-error-line': { backgroundColor: 'color-mix(in oklch, var(--destructive) 12%, transparent)' },
  '.cm-query-error': { textDecoration: 'underline wavy var(--destructive)', textDecorationSkipInk: 'none' },
});

interface Props {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  error: QueryError | null;
  placeholder?: string;
  /** Accessible name of the editable area. */
  label?: string;
  onCreateEditor?: (view: EditorView) => void;
  className?: string;
}

/**
 * BQL editor (CodeMirror): Cmd/Ctrl+Enter runs, the server's error position is underlined. Font size comes from the
 * container (`text-base md:text-sm`: 16px on phones so iOS does not zoom on focus).
 */
export default function QueryEditor({ value, onChange, onRun, error, placeholder, label, onCreateEditor, className }: Props) {
  const { resolvedTheme } = useTheme();
  const [view, setView] = useState<EditorView | null>(null);
  const onRunRef = useRef(onRun);
  onRunRef.current = onRun;

  const extensions = useMemo(
    () => [
      Prec.highest(
        keymap.of([
          {
            key: 'Mod-Enter',
            run: () => {
              onRunRef.current();
              return true;
            },
          },
        ]),
      ),
      bqlHighlight,
      errorField,
      queryEditorTheme,
      EditorView.lineWrapping,
      EditorView.contentAttributes.of(label ? { 'aria-label': label } : {}),
    ],
    [label],
  );

  useEffect(() => {
    if (!view) return;
    const range = error ? errorRangeOf(view.state.doc, error.line, error.column) : null;
    view.dispatch({ effects: setErrorRange.of(range) });
  }, [view, error]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      extensions={extensions}
      placeholder={placeholder}
      theme={resolvedTheme === 'dark' ? 'dark' : 'light'}
      className={cn(
        '[&_.cm-editor]:bg-transparent! [&_.cm-placeholder]:text-muted-foreground!',
        '[&_.cm-gutters]:border-r! [&_.cm-gutters]:border-border! [&_.cm-gutters]:bg-transparent! [&_.cm-gutters]:text-muted-foreground!',
        '[&_.cm-activeLine]:bg-muted/40! [&_.cm-activeLineGutter]:bg-muted!',
        className,
      )}
      minHeight="120px"
      maxHeight="45vh"
      basicSetup={{ foldGutter: false, autocompletion: false }}
      onCreateEditor={(editorView) => {
        setView(editorView);
        onCreateEditor?.(editorView);
      }}
    />
  );
}
