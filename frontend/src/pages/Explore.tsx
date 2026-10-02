import { executeQuery } from '@/api/requests';
import { QueryError, QueryResult } from '@/api/types';
import QueryEditor from '@/components/query/QueryEditor';
import { errorRangeOf } from '@/components/query/errorRange';
import QueryReference from '@/components/query/QueryReference';
import QueryResultTable from '@/components/query/QueryResultTable';
import { DEFAULT_QUERY, QUERY_EXAMPLES } from '@/components/query/examples';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { QUERY_LINK } from '@/layout/Sidebar';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { EditorView } from '@uiw/react-codemirror';
import { useAtomValue, useSetAtom } from 'jotai';
import { ChevronDown, CircleAlert, LoaderCircle, Play } from 'lucide-react';
import { ApiError } from 'openapi-typescript-fetch';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { breadcrumbAtom, titleAtom } from '../states/basic';

const RUN_SHORTCUT = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent) ? '⌘ Enter' : 'Ctrl Enter';

interface QueryOutcome {
  id: number;
  result: QueryResult;
  elapsedMs: number;
}

function toQueryError(e: unknown): QueryError {
  if (e instanceof ApiError) {
    const data = typeof e.data === 'object' && e.data !== null ? e.data : {};
    return {
      message: typeof data.message === 'string' ? data.message : `${e.status} ${e.statusText}`,
      line: typeof data.line === 'number' ? data.line : null,
      column: typeof data.column === 'number' ? data.column : null,
    };
  }
  return { message: e instanceof Error ? e.message : String(e), line: null, column: null };
}

export default function Explore() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);

  useDocumentTitle(`Query - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([QUERY_LINK]);
  }, []);

  const [query, setQuery] = useLocalStorage({ key: 'query-explore-query', defaultValue: DEFAULT_QUERY, getInitialValueInEffect: false });
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<QueryOutcome | null>(null);
  const [error, setError] = useState<QueryError | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const runningRef = useRef(false);

  const runQuery = async (text: string) => {
    if (runningRef.current || text.trim() === '') return;
    runningRef.current = true;
    setRunning(true);
    const startedAt = performance.now();
    try {
      // the query is sent verbatim so that error positions match the editor content
      const res = await executeQuery({ query: text });
      const elapsedMs = Math.round(performance.now() - startedAt);
      setOutcome((prev) => ({ id: (prev?.id ?? 0) + 1, result: res.data.data, elapsedMs }));
      setError(null);
    } catch (e) {
      setOutcome(null);
      setError(toQueryError(e));
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  };

  const runCurrent = () => runQuery(viewRef.current?.state.doc.toString() ?? query);

  const runExample = (exampleQuery: string) => {
    setQuery(exampleQuery);
    runQuery(exampleQuery);
  };

  const insertText = (text: string, cursorBack = 0) => {
    const view = viewRef.current;
    if (!view) return;
    const { from, to } = view.state.selection.main;
    view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length - cursorBack }, scrollIntoView: true });
    view.focus();
  };

  const jumpToError = () => {
    const view = viewRef.current;
    if (!view || !error) return;
    const range = errorRangeOf(view.state.doc, error.line, error.column);
    if (!range) return;
    view.dispatch({ selection: { anchor: range.from }, scrollIntoView: true });
    view.focus();
  };

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-xl font-semibold tracking-tight">{t('NAV_QUERY')}</h1>
        <div className="flex items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm">
                {t('query.examples')}
                <ChevronDown className="ml-2 h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[min(28rem,calc(100vw-2rem))]">
              {QUERY_EXAMPLES.map((example) => (
                <DropdownMenuItem key={example.title} className="flex flex-col items-start gap-1" onSelect={() => runExample(example.query)}>
                  <span className="font-medium">{t(example.title)}</span>
                  <code className="line-clamp-2 text-xs text-muted-foreground">{example.query}</code>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <QueryReference onInsert={insertText} />
        </div>
      </div>

      <div className="overflow-hidden rounded-md border">
        <QueryEditor
          value={query}
          onChange={setQuery}
          onRun={runCurrent}
          error={error}
          placeholder={t('query.placeholder')}
          onCreateEditor={(view) => {
            viewRef.current = view;
          }}
        />
      </div>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <Button onClick={runCurrent} disabled={running}>
          {running ? <LoaderCircle className="mr-2 h-4 w-4 animate-spin" /> : <Play className="mr-2 h-4 w-4" />}
          {running ? t('query.running') : t('query.run')}
        </Button>
        <span className="hidden text-xs text-muted-foreground sm:inline">
          {t('query.run_hint')} <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono">{RUN_SHORTCUT}</kbd>
        </span>
        {outcome && (
          <span className="ml-auto text-sm text-muted-foreground">
            {t('query.rows', { count: outcome.result.rows.length })} · {t('query.elapsed', { ms: outcome.elapsedMs })}
          </span>
        )}
      </div>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/5 p-3 text-sm">
          <div className="flex flex-wrap items-center gap-2 font-medium text-destructive">
            <CircleAlert className="h-4 w-4" />
            {t('query.error_title')}
            {error.line !== null && (
              <button type="button" className="text-xs font-normal underline underline-offset-2" onClick={jumpToError}>
                {error.column !== null ? t('query.error_position', { line: error.line, column: error.column }) : t('query.error_line', { line: error.line })}
              </button>
            )}
          </div>
          <pre className="mt-2 whitespace-pre-wrap break-words font-mono text-xs text-destructive">{error.message}</pre>
        </div>
      )}

      {outcome && <QueryResultTable key={outcome.id} result={outcome.result} />}

      {!outcome && !error && <p className="text-sm text-muted-foreground">{t('query.empty_hint')}</p>}
    </div>
  );
}
