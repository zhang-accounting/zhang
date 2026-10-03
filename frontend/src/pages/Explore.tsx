import { EditorView } from '@uiw/react-codemirror';
import { useAtomValue, useSetAtom } from 'jotai';
import { CircleAlert, Crosshair, DatabaseZap, Download, Play } from 'lucide-react';
import { ApiError } from 'openapi-typescript-fetch';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { useAsync } from 'react-use';
import { executeQuery, exportQueryCsv, retrieveOptions } from '@/api/requests';
import { QueryError, QueryResult } from '@/api/types';
import { EmptyState, PageHeader, PageShell } from '@/components/layout';
import QueryEditor from '@/components/query/QueryEditor';
import { errorRangeOf } from '@/components/query/errorRange';
import QueryReference from '@/components/query/QueryReference';
import QueryResults from '@/components/query/QueryResults';
import SavedQueriesMenu from '@/components/query/SavedQueriesMenu';
import { DEFAULT_QUERY, QUERY_EXAMPLES } from '@/components/query/examples';
import { EXPLORE_QUERY_PARAM, queryFromSearch } from '@/components/query/explore-link';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Kbd } from '@/components/ui/kbd';
import { Spinner } from '@/components/ui/spinner';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { QUERY_LINK } from '@/layout/nav-links';
import { cn } from '@/lib/utils';
import { breadcrumbAtom, titleAtom } from '@/states/basic';

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent);
const RUN_SHORTCUT = IS_MAC ? '⌘ ↵' : 'Ctrl ↵';

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

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  // revoked a little later, as some browsers start the download asynchronously
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** A query (or CSV export) error from the server; the position button moves the editor cursor to it. */
function QueryErrorAlert({ title, error, onJump }: { title: string; error: QueryError; onJump: (error: QueryError) => void }) {
  const { t } = useTranslation();
  return (
    <div role="alert" className="flex gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive md:p-4">
      <CircleAlert className="mt-0.5 size-4 shrink-0" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-x-3">
          <span className="font-medium">{title}</span>
          {error.line !== null && (
            <Button
              variant="link"
              size="sm"
              className="h-10 px-0 text-destructive underline md:h-6"
              onClick={() => onJump(error)}
              title={t('query.error_jump')}
            >
              <Crosshair />
              {error.column !== null ? t('query.error_position', { line: error.line, column: error.column }) : t('query.error_line', { line: error.line })}
            </Button>
          )}
        </div>
        <pre className="font-mono text-xs break-words whitespace-pre-wrap">{error.message}</pre>
      </div>
    </div>
  );
}

export default function Explore() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);

  useDocumentTitle(`${t('NAV_QUERY')} - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([QUERY_LINK]);
  }, [setBreadcrumb]);

  const [query, setQuery] = useLocalStorage({ key: 'query-explore-query', defaultValue: DEFAULT_QUERY });
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<QueryOutcome | null>(null);
  const [error, setError] = useState<QueryError | null>(null);
  const [exporting, setExporting] = useState(false);
  // kept apart from `error`, so a failed export leaves the result on screen
  const [exportError, setExportError] = useState<QueryError | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const runningRef = useRef(false);

  const runQuery = async (text: string) => {
    if (runningRef.current || text.trim() === '') return;
    runningRef.current = true;
    setRunning(true);
    setExportError(null);
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

  const { value: operatingCurrency } = useAsync(async () => {
    const res = await retrieveOptions({});
    return res.data.data.find((option) => option.key === 'operating_currency')?.value.trim() || undefined;
  }, []);

  const currentQuery = () => viewRef.current?.state.doc.toString() ?? query;

  const runCurrent = () => runQuery(currentQuery());

  const exportCsv = async () => {
    const text = currentQuery();
    if (exporting || text.trim() === '') return;
    setExporting(true);
    setExportError(null);
    try {
      const { blob, filename } = await exportQueryCsv(text);
      downloadBlob(blob, filename);
    } catch (e) {
      setExportError(toQueryError(e));
    } finally {
      setExporting(false);
    }
  };

  /** Examples and saved queries: load into the editor and run. */
  const loadAndRun = (text: string) => {
    setQuery(text);
    runQuery(text);
  };

  // a query in the URL (`/explore?query=...`, e.g. from "Open in Explore") is what the user asked to see: put it in the
  // editor and run it. It then leaves the URL, so a reload or going back does not replace what the user typed since.
  const [searchParams, setSearchParams] = useSearchParams();
  const urlQuery = queryFromSearch(searchParams);
  const openedQueryRef = useRef<string | null>(null);
  useEffect(() => {
    if (urlQuery === null) {
      openedQueryRef.current = null;
      return;
    }
    setSearchParams(
      (params) => {
        params.delete(EXPLORE_QUERY_PARAM);
        return params;
      },
      { replace: true },
    );
    // React's development double effects must not run it twice
    if (openedQueryRef.current === urlQuery) return;
    openedQueryRef.current = urlQuery;
    loadAndRun(urlQuery);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only a new query in the URL loads one
  }, [urlQuery]);

  const insertText = (text: string, cursorBack = 0) => {
    const view = viewRef.current;
    if (!view) return;
    const { from, to } = view.state.selection.main;
    // a word inserted right after another token is kept apart from it, e.g. `SELECT *` + `FROM #prices`
    const before = view.state.sliceDoc(Math.max(0, from - 1), from);
    const insert = /^[\w#]/.test(text) && /[\w*)'"]/.test(before) ? ` ${text}` : text;
    view.dispatch({ changes: { from, to, insert }, selection: { anchor: from + insert.length - cursorBack }, scrollIntoView: true });
    view.focus();
  };

  const jumpToError = (target: QueryError) => {
    const view = viewRef.current;
    if (!view) return;
    const range = errorRangeOf(view.state.doc, target.line, target.column);
    if (!range) return;
    view.dispatch({ selection: { anchor: range.from }, scrollIntoView: true });
    view.focus();
  };

  const emptyQuery = query.trim() === '';

  return (
    <PageShell>
      <PageHeader
        title={t('NAV_QUERY')}
        description={t('query.description')}
        actions={
          <>
            <SavedQueriesMenu onSelect={loadAndRun} />
            <QueryReference onInsert={insertText} />
          </>
        }
      />

      <Card size="sm" className="gap-0 py-0 has-[.cm-focused]:ring-2 has-[.cm-focused]:ring-ring">
        <QueryEditor
          value={query}
          onChange={setQuery}
          onRun={runCurrent}
          error={error ?? exportError}
          placeholder={t('query.placeholder')}
          label={t('query.editor_label')}
          className="text-base md:text-sm"
          onCreateEditor={(view) => {
            viewRef.current = view;
          }}
        />
        <div className="flex flex-col gap-2 border-t bg-muted/40 px-3 py-2 md:flex-row md:items-center">
          <div
            role="group"
            aria-label={t('query.examples')}
            // one swipeable row on phones (no scrollbar), wrapping chips from md up
            className={cn(
              'flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
              'md:flex-wrap md:overflow-visible',
            )}
          >
            <span className="shrink-0 pr-1 text-xs text-muted-foreground">{t('query.examples')}</span>
            {QUERY_EXAMPLES.map((example) => (
              <Button
                key={example.title}
                variant="ghost"
                size="sm"
                className="h-10 shrink-0 text-xs md:h-7"
                title={example.query}
                disabled={running}
                onClick={() => loadAndRun(example.query)}
              >
                {t(example.title)}
              </Button>
            ))}
          </div>
          <div className="grid grid-cols-2 gap-2 md:flex md:shrink-0 md:self-start">
            <Button variant="outline" className="h-10 md:h-8" onClick={exportCsv} disabled={exporting || emptyQuery}>
              {exporting ? <Spinner aria-hidden /> : <Download />}
              {exporting ? t('query.exporting_csv') : t('query.export_csv')}
            </Button>
            <Button className="h-10 md:h-8" onClick={runCurrent} disabled={running || emptyQuery} aria-keyshortcuts="Meta+Enter Control+Enter">
              {running ? <Spinner aria-hidden /> : <Play />}
              {running ? t('query.running') : t('query.run')}
              <Kbd className="hidden bg-primary-foreground/15 text-primary-foreground md:inline-flex">{RUN_SHORTCUT}</Kbd>
            </Button>
          </div>
        </div>
      </Card>

      {exportError && <QueryErrorAlert title={t('query.export_error_title')} error={exportError} onJump={jumpToError} />}

      {error && <QueryErrorAlert title={t('query.error_title')} error={error} onJump={jumpToError} />}

      {outcome && (
        <QueryResults runId={outcome.id} result={outcome.result} elapsedMs={outcome.elapsedMs} stale={running} operatingCurrency={operatingCurrency} />
      )}

      {!outcome && !error && <EmptyState icon={DatabaseZap} title={t('query.empty_title')} description={t('query.empty_hint')} />}
    </PageShell>
  );
}
