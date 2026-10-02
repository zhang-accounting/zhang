import { ChartColumn, Table2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { QueryResult } from '@/api/types';
import { detectChartKind } from '@/components/query/chartData';
import QueryResultChart from '@/components/query/QueryResultChart';
import QueryResultTable from '@/components/query/QueryResultTable';
import { Card, CardAction, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { cn } from '@/lib/utils';

interface Props {
  /** Changes with every run: resets the table's "show all" state. */
  runId: number;
  result: QueryResult;
  /** Round-trip time of the request that produced `result`. */
  elapsedMs: number;
  /** A new query is running: the stale result is dimmed. */
  stale?: boolean;
  /** The ledger's operating currency, plotted by default when the result has it. */
  operatingCurrency?: string;
}

/**
 * Results card: row count and timing, plus a Table / Chart switch when the result can be charted (a label or date column
 * followed by one value column, or by several such as a PIVOT BY result, see `detectChartKind`). The choice is
 * remembered; results that cannot be charted show the table.
 */
export default function QueryResults({ runId, result, elapsedMs, stale = false, operatingCurrency }: Props) {
  const { t } = useTranslation();
  const chartKind = useMemo(() => detectChartKind(result), [result]);
  const [showChart, setShowChart] = useLocalStorage({ key: 'query-explore-show-chart', defaultValue: true });
  const [pickedCurrency, setPickedCurrency] = useState<string | undefined>(undefined);
  const view = chartKind && showChart ? 'chart' : 'table';

  return (
    <Card className={cn('gap-0 pb-0 transition-opacity', stale && 'opacity-60')} aria-busy={stale}>
      <Tabs value={view} onValueChange={(value) => setShowChart(value === 'chart')} className="gap-0">
        <CardHeader className="border-b">
          <CardTitle className="flex items-center gap-2">
            {view === 'chart' ? <ChartColumn className="size-4 text-muted-foreground" /> : <Table2 className="size-4 text-muted-foreground" />}
            {t('query.results')}
          </CardTitle>
          <CardDescription className="flex flex-wrap gap-x-3 tabular-nums">
            <span>{t('query.rows', { count: result.rows.length })}</span>
            <span>{t('query.elapsed', { ms: elapsedMs })}</span>
          </CardDescription>
          {chartKind && (
            <CardAction>
              <TabsList aria-label={t('query.view')}>
                <TabsTrigger value="table" className="px-2.5">
                  <Table2 />
                  {t('query.show_table')}
                </TabsTrigger>
                <TabsTrigger value="chart" className="px-2.5">
                  <ChartColumn />
                  {t('query.show_chart')}
                </TabsTrigger>
              </TabsList>
            </CardAction>
          )}
        </CardHeader>
        <TabsContent value="table" className="min-w-0">
          <QueryResultTable key={runId} result={result} />
        </TabsContent>
        {chartKind && (
          <TabsContent value="chart" className="min-w-0">
            <QueryResultChart
              result={result}
              kind={chartKind}
              operatingCurrency={operatingCurrency}
              pickedCurrency={pickedCurrency}
              onPickCurrency={setPickedCurrency}
            />
          </TabsContent>
        )}
      </Tabs>
    </Card>
  );
}
