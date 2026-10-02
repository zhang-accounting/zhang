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
import { useEffect, useMemo, useRef, useState } from 'react';

// Lightweight BQL highlighting built on the view package only, since no CodeMirror language package is installed.
// Groups: string, date, number, `#table`, keyword, function name.
const TOKEN_REGEXP =
  /("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?)|\b(\d{4}-\d{2}-\d{2})\b|\b(\d+(?:\.\d+)?)\b|(?<![\w#])(#[a-z_][a-z0-9_]*)|\b(select|distinct|from|where|group|by|order|asc|desc|limit|as|and|or|not|in|is|null|true|false|open|close|on|clear|balances|journal|at|pivot|having)\b|\b([a-z_][a-z0-9_]*)(?=\s*\()/gi;

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

const queryEditorTheme = EditorView.baseTheme({
  '&': { fontSize: '13px' },
  '.cm-content': { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' },
  '&.cm-focused': { outline: 'none' },
  '.cm-bql-keyword': { color: '#7c3aed', fontWeight: '600' },
  '.cm-bql-function': { color: '#0369a1' },
  '.cm-bql-table': { color: '#0f766e', fontWeight: '600' },
  '.cm-bql-string': { color: '#047857' },
  '.cm-bql-number': { color: '#1d4ed8' },
  '.cm-bql-date': { color: '#b45309' },
  '.cm-query-error-line': { backgroundColor: 'rgba(220, 38, 38, 0.08)' },
  '.cm-query-error': { textDecoration: 'underline wavy #dc2626', textDecorationSkipInk: 'none' },
});

interface Props {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  error: QueryError | null;
  placeholder?: string;
  onCreateEditor?: (view: EditorView) => void;
}

export default function QueryEditor({ value, onChange, onRun, error, placeholder, onCreateEditor }: Props) {
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
    ],
    [],
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
