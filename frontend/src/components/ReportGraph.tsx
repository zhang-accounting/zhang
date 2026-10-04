import { ChartNoAxesColumn } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { Bar, BarChart, CartesianGrid, Line, LineChart, ReferenceLine, XAxis, YAxis } from 'recharts';
import { GraphRow, useAxisFormatter } from '@/components/layout/chart-utils';
import { cn } from '@/lib/utils';
import Amount from './Amount';
import { ChartConfig, ChartContainer, ChartLegend, ChartLegendContent, ChartTooltip, ChartTooltipContent } from './ui/chart';

// Income = chart-1 (teal), expenses = chart-2 (caramel) everywhere; single-series trends use chart-1 (the turquoise
// primary is a fill colour, too light for a 2px line on light surfaces).
const INCOME_COLOR = 'var(--chart-1)';
const EXPENSE_COLOR = 'var(--chart-2)';
const TREND_COLOR = 'var(--chart-1)';

function TooltipRow({ color, label, value, commodity }: { color: string; label: React.ReactNode; value: number; commodity: string }) {
  return (
    <div className="flex w-full items-center gap-2">
      <span className="size-2.5 shrink-0 rounded-[2px]" style={{ backgroundColor: color }} />
      <span className="flex-1 text-muted-foreground">{label}</span>
      <Amount className="font-medium text-foreground" amount={value} currency={commodity} />
    </div>
  );
}

function ChartEmpty({ className, children }: { className?: string; children: React.ReactNode }) {
  return (
    <div className={cn('flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed text-sm text-muted-foreground', className)}>
      <ChartNoAxesColumn className="size-5" aria-hidden />
      {children}
    </div>
  );
}

interface ChartProps {
  rows: GraphRow[];
  commodity: string;
  /** Height utilities, e.g. `h-48 md:h-64`. */
  className?: string;
}

/** Net worth over time: one series, one axis, y-domain fitted to the data so small movements stay visible. */
export function BalanceTrendChart({ rows, commodity, className }: ChartProps) {
  const { t } = useTranslation();
  const values = React.useMemo(() => rows.map((row) => row.total), [rows]);
  const axis = useAxisFormatter(values);
  const config = { total: { label: t('ledger.chart.net_worth'), color: TREND_COLOR } } satisfies ChartConfig;

  if (rows.length === 0) return <ChartEmpty className={cn('h-56', className)}>{t('ledger.chart.no_data')}</ChartEmpty>;

  return (
    <ChartContainer config={config} className={cn('aspect-auto h-56 w-full', className)}>
      <LineChart accessibilityLayer data={rows} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
        <CartesianGrid vertical={false} />
        <XAxis dataKey="label" tickLine={false} axisLine={false} tickMargin={8} minTickGap={28} />
        <YAxis width={axis.width} tickLine={false} axisLine={false} ticks={axis.ticks} domain={axis.domain} tickFormatter={axis.format} />
        <ChartTooltip
          cursor={{ strokeDasharray: '3 3' }}
          content={
            <ChartTooltipContent
              labelFormatter={(_, payload) => payload?.[0]?.payload?.fullLabel}
              formatter={(value) => <TooltipRow color="var(--color-total)" label={config.total.label} value={Number(value)} commodity={commodity} />}
            />
          }
        />
        <Line dataKey="total" type="monotone" stroke="var(--color-total)" strokeWidth={2} dot={false} activeDot={{ r: 4 }} isAnimationActive={false} />
      </LineChart>
    </ChartContainer>
  );
}

/** Income above / expenses below a shared zero line (one axis, two hues). */
export function CashFlowChart({ rows, commodity, className }: ChartProps) {
  const { t } = useTranslation();
  const values = React.useMemo(() => rows.flatMap((row) => [row.income, row.expense]), [rows]);
  const axis = useAxisFormatter(values);
  const config = {
    income: { label: t('ledger.chart.income'), color: INCOME_COLOR },
    expense: { label: t('ledger.chart.expenses'), color: EXPENSE_COLOR },
  } satisfies ChartConfig;

  if (!rows.some((row) => row.income !== 0 || row.expense !== 0)) {
    return <ChartEmpty className={cn('h-56', className)}>{t('ledger.chart.no_cash_flow')}</ChartEmpty>;
  }

  return (
    <ChartContainer config={config} className={cn('aspect-auto h-56 w-full', className)}>
      <BarChart accessibilityLayer data={rows} stackOffset="sign" margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
        <CartesianGrid vertical={false} />
        <XAxis dataKey="label" tickLine={false} axisLine={false} tickMargin={8} minTickGap={28} />
        <YAxis width={axis.width} tickLine={false} axisLine={false} ticks={axis.ticks} domain={axis.domain} tickFormatter={axis.format} />
        <ReferenceLine y={0} stroke="var(--border)" />
        <ChartTooltip
          cursor={{ fill: 'var(--muted)', opacity: 0.6 }}
          content={
            <ChartTooltipContent
              labelFormatter={(_, payload) => payload?.[0]?.payload?.fullLabel}
              formatter={(value, name) => (
                <TooltipRow
                  color={`var(--color-${name})`}
                  label={name === 'income' ? config.income.label : config.expense.label}
                  value={Math.abs(Number(value))}
                  commodity={commodity}
                />
              )}
            />
          }
        />
        <ChartLegend content={<ChartLegendContent />} itemSorter={(item) => (item.dataKey === 'income' ? 0 : 1)} />
        <Bar dataKey="income" stackId="flow" fill="var(--color-income)" radius={[3, 3, 0, 0]} maxBarSize={28} isAnimationActive={false} />
        <Bar dataKey="expense" stackId="flow" fill="var(--color-expense)" radius={[3, 3, 0, 0]} maxBarSize={28} isAnimationActive={false} />
      </BarChart>
    </ChartContainer>
  );
}
