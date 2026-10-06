import BigNumber from 'bignumber.js';
import { parseISO } from 'date-fns';
import { sortBy } from 'lodash-es';
import { LineChartIcon } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from 'recharts';
import { EmptyState } from '@/components/layout';
import { spansYears, useDateFormat } from '@/components/layout/use-date-format';
import { cn } from '@/lib/utils';
import type { BalancePoint } from '@/utils/account-history';
import Amount from './Amount';
import { useAxisFormatter } from '@/components/layout/chart-utils';
import { ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from './ui/chart';
import { Toggle } from './ui/toggle';

interface Props {
  /** the balance at the end of every day with a posting, per commodity (`balanceHistoryByCommodity`) */
  data?: Record<string, BalancePoint[]>;
  /** Height utilities, e.g. `h-56 md:h-80`. */
  className?: string;
}

/**
 * Balance of one commodity over (real) time. The balance only changes on posting days, so the line is drawn as a step.
 * Accounts holding several commodities get a commodity switch instead of mixing units on one axis.
 */
export function AccountBalanceHistoryGraph({ data, className }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const commodities = useMemo(() => Object.keys(data ?? {}).sort(), [data]);
  const [selected, setSelected] = useState<string | undefined>(undefined);
  const commodity = selected && commodities.includes(selected) ? selected : commodities[0];

  const points = useMemo(() => {
    if (!data || !commodity) return [];
    return sortBy(
      (data[commodity] ?? []).map((it) => ({ ts: parseISO(it.date).getTime(), balance: new BigNumber(it.balance.number).toNumber() })),
      (it) => it.ts,
    );
  }, [data, commodity]);

  const values = useMemo(() => points.map((it) => it.balance), [points]);
  // `Sep 13` is ambiguous once the history covers several years: show `Sep 13, 2023` then.
  const multiYear = useMemo(() => spansYears(points.map((it) => it.ts)), [points]);
  const axis = useAxisFormatter(values);

  if (points.length === 0) {
    return <EmptyState icon={LineChartIcon} title={t('ledger.account.no_history')} description={t('ledger.account.no_history_description')} />;
  }

  const config = { balance: { label: commodity, color: 'var(--chart-1)' } } satisfies ChartConfig;

  return (
    <div className="flex flex-col gap-3">
      {commodities.length > 1 && (
        <div className="flex flex-wrap gap-1.5" role="group" aria-label={t('ledger.account.commodity')}>
          {commodities.map((it) => (
            <Toggle key={it} variant="outline" size="sm" className="h-10 min-w-12 md:h-7" pressed={it === commodity} onPressedChange={() => setSelected(it)}>
              {it}
            </Toggle>
          ))}
        </div>
      )}
      <ChartContainer config={config} className={cn('aspect-auto h-56 w-full md:h-80', className)}>
        <LineChart accessibilityLayer data={points} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid vertical={false} />
          <XAxis
            dataKey="ts"
            type="number"
            scale="time"
            domain={['dataMin', 'dataMax']}
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            minTickGap={32}
            tickFormatter={(value: number) => (multiYear ? fmt.date(value) : fmt.day(value))}
          />
          <YAxis width={axis.width} tickLine={false} axisLine={false} ticks={axis.ticks} domain={axis.domain} tickFormatter={axis.format} />
          <ChartTooltip
            cursor={{ strokeDasharray: '3 3' }}
            content={
              <ChartTooltipContent
                labelFormatter={(_, payload) => {
                  const ts = payload?.[0]?.payload?.ts;
                  return ts ? fmt.weekdayDate(ts) : '';
                }}
                formatter={(value) => (
                  <div className="flex w-full items-center gap-2">
                    <span className="size-2.5 shrink-0 rounded-[2px] bg-(--color-balance)" />
                    <span className="flex-1 text-muted-foreground">{t('ledger.account.balance')}</span>
                    <Amount className="font-medium text-foreground" amount={Number(value)} currency={commodity} />
                  </div>
                )}
              />
            }
          />
          <Line dataKey="balance" type="stepAfter" stroke="var(--color-balance)" strokeWidth={2} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
        </LineChart>
      </ChartContainer>
    </div>
  );
}
