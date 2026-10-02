import { executeQuery, exportQueryCsv, retrieveOptions } from '@/api/requests';
import { QueryError, QueryResult } from '@/api/types';
import { detectChartKind } from '@/components/query/chartData';
import QueryEditor from '@/components/query/QueryEditor';
import { errorRangeOf } from '@/components/query/errorRange';
import QueryReference from '@/components/query/QueryReference';
import QueryResultChart from '@/components/query/QueryResultChart';
import QueryResultTable from '@/components/query/QueryResultTable';
import SavedQueriesMenu from '@/components/query/SavedQueriesMenu';
import { DEFAULT_QUERY, QUERY_EXAMPLES } from '@/components/query/examples';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { QUERY_LINK } from '@/layout/Sidebar';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { EditorView } from '@uiw/react-codemirror';
import { useAtomValue, useSetAtom } from 'jotai';
import { ChevronDown, CircleAlert, Download, LoaderCircle, Play } from 'lucide-react';
import { ApiError } from 'openapi-typescript-fetch';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
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

function QueryErrorBox({ title, error, onJump }: { title: string; error: QueryError; onJump: (error: QueryError) => void }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-md border border-destructive/50 bg-destructive/5 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2 font-medium text-destructive">
        <CircleAlert className="h-4 w-4" />
        {title}
        {error.line !== null && (
          <button type="button" className="text-xs font-normal underline underline-offset-2" onClick={() => onJump(error)}>
            {error.column !== null ? t('query.error_position', { line: error.line, column: error.column }) : t('query.error_line', { line: error.line })}
          </button>
        )}
      </div>
      <pre className="mt-2 whitespace-pre-wrap break-words font-mono text-xs text-destructive">{error.message}</pre>
    </div>
  );
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
  const [exporting, setExporting] = useState(false);
  // kept apart from `error`, so a failed export leaves the result on screen
  const [exportError, setExportError] = useState<QueryError | null>(null);
  const [showChart, setShowChart] = useLocalStorage({ key: 'query-explore-show-chart', defaultValue: true, getInitialValueInEffect: false });
  const viewRef = useRef<EditorView | null>(null);
  const runningRef = useRef(false);

  const { value: operatingCurrency } = useAsync(async () => {
    const res = await retrieveOptions({});
    return res.data.data.find((option) => option.key === 'operating_currency')?.value.trim() || undefined;
  }, []);
  const chartKind = useMemo(() => (outcome ? detectChartKind(outcome.result) : null), [outcome]);

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

  const loadAndRun = (text: string) => {
    setQuery(text);
    runQuery(text);
  };

  const insertText = (text: string, cursorBack = 0) => {
    const view = viewRef.current;
    if (!view) return;
    const { from, to } = view.state.selection.main;
    view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length - cursorBack }, scrollIntoView: true });
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

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-xl font-semibold tracking-tight">{t('NAV_QUERY')}</h1>
        <div className="flex flex-wrap items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm">
                {t('query.examples')}
                <ChevronDown className="ml-2 h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[min(28rem,calc(100vw-2rem))]">
              {QUERY_EXAMPLES.map((example) => (
                <DropdownMenuItem key={example.title} className="flex flex-col items-start gap-1" onSelect={() => loadAndRun(example.query)}>
                  <span className="font-medium">{t(example.title)}</span>
                  <code className="line-clamp-2 text-xs text-muted-foreground">{example.query}</code>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <SavedQueriesMenu onSelect={loadAndRun} />
          <QueryReference onInsert={insertText} />
        </div>
      </div>

      <div className="overflow-hidden rounded-md border">
        <QueryEditor
          value={query}
          onChange={setQuery}
          onRun={runCurrent}
          error={error ?? exportError}
          placeholder={t('query.placeholder')}
          onCreateEditor={(view) => {
            viewRef.current = view;
          }}
        />
      </div>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <div className="flex items-center gap-2">
          <Button onClick={runCurrent} disabled={running}>
            {running ? <LoaderCircle className="mr-2 h-4 w-4 animate-spin" /> : <Play className="mr-2 h-4 w-4" />}
            {running ? t('query.running') : t('query.run')}
          </Button>
          <Button variant="outline" onClick={exportCsv} disabled={exporting}>
            {exporting ? <LoaderCircle className="mr-2 h-4 w-4 animate-spin" /> : <Download className="mr-2 h-4 w-4" />}
            {exporting ? t('query.exporting_csv') : t('query.export_csv')}
          </Button>
        </div>
        <span className="hidden text-xs text-muted-foreground sm:inline">
          {t('query.run_hint')} <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono">{RUN_SHORTCUT}</kbd>
        </span>
        {outcome && (
          <div className="ml-auto flex items-center gap-4">
            {chartKind && (
              <div className="flex items-center gap-2">
                <Switch id="query-show-chart" checked={showChart} onCheckedChange={setShowChart} />
                <Label htmlFor="query-show-chart" className="cursor-pointer">
                  {t('query.show_chart')}
                </Label>
              </div>
            )}
            <span className="text-sm text-muted-foreground">
              {t('query.rows', { count: outcome.result.rows.length })} · {t('query.elapsed', { ms: outcome.elapsedMs })}
            </span>
          </div>
        )}
      </div>

      {exportError && <QueryErrorBox title={t('query.export_error_title')} error={exportError} onJump={jumpToError} />}

      {error && <QueryErrorBox title={t('query.error_title')} error={error} onJump={jumpToError} />}

      {outcome && chartKind && showChart && <QueryResultChart result={outcome.result} kind={chartKind} operatingCurrency={operatingCurrency} />}

      {outcome && <QueryResultTable key={outcome.id} result={outcome.result} />}

      {!outcome && !error && <p className="text-sm text-muted-foreground">{t('query.empty_hint')}</p>}
    </div>
  );
}
