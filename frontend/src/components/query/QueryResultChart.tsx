import { QueryResult } from '@/api/types';
import {
  BarDatum,
  buildBars,
  buildLine,
  buildTreemap,
  ChartPoint,
  collectPoints,
  currenciesOf,
  defaultCurrency,
  LineDatum,
  NO_CURRENCY,
  QueryChartKind,
  TreemapDatum,
} from '@/components/query/chartData';
import { formatDecimal } from '@/components/query/values';
import { ChartConfig, ChartContainer, ChartStyle, ChartTooltip } from '@/components/ui/chart';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { useMediaQuery } from '@mantine/hooks';
import BigNumber from 'bignumber.js';
import { eachDayOfInterval, eachMonthOfInterval, eachYearOfInterval, format } from 'date-fns';
import { ReactNode, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Bar, BarChart, CartesianGrid, Cell, Line, LineChart, ReferenceLine, TooltipProps, Treemap, XAxis, YAxis } from 'recharts';

// Categorical slots 1 (blue) and 2 (orange), stepped separately for the light and dark surfaces. Blue plots values,
// orange marks negative values in treemaps and bar charts, where the area/length is the absolute value or the sign
// would otherwise be easy to miss.
const chartConfig = {
  value: { theme: { light: '#2a78d6', dark: '#3987e5' } },
  negative: { theme: { light: '#eb6834', dark: '#d95926' } },
} satisfies ChartConfig;

const MAX_BARS = 50;
const BAR_HEIGHT = 28;
/** Average glyph width of the 12px chart labels, used to fit labels into the space they get. */
const CHAR_WIDTH = 6.5;

const compactNumber = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });

/** The exact signed value as the table shows it, trailing zeros included. */
function formatExact(signed: string, currency: string): string {
  const number = formatDecimal(signed);
  return currency === NO_CURRENCY ? number : `${number} ${currency}`;
}

function truncate(text: string, maxChars: number): string {
  if (maxChars < 2) return '';
  return text.length > maxChars ? `${text.slice(0, maxChars - 1)}…` : text;
}

function TooltipBox({ title, value, negative }: { title: string; value: string; negative?: boolean }) {
  return (
    <div className="grid max-w-xs gap-1 rounded-lg border border-border/50 bg-background px-2.5 py-1.5 text-xs shadow-xl">
      <div className="break-all font-medium">{title}</div>
      <div className="flex items-center gap-1.5">
        <span className="h-2.5 w-2.5 shrink-0 rounded-[2px]" style={{ backgroundColor: negative ? 'var(--color-negative)' : 'var(--color-value)' }} />
        <span className="font-mono tabular-nums text-foreground">{value}</span>
      </div>
    </div>
  );
}

function LegendItem({ color, label }: { color: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className="h-2.5 w-2.5 rounded-[2px]" style={{ backgroundColor: color }} />
      {label}
    </span>
  );
}

// ---- treemap ----

interface TreemapCellProps {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  depth?: number;
  name?: string;
  negative?: boolean;
  signed?: string;
  children?: unknown[] | null;
  root?: { x: number; y: number; width: number; height: number; depth: number };
}

/**
 * Draws a leaf cell. Group nodes are not drawn: their leaves cover them. Leaves are inset to leave a 2px gap, and
 * twice that along the border of their parent account, so sibling groups read as groups.
 */
function TreemapCell({ x = 0, y = 0, width = 0, height = 0, depth = 0, name = '', negative, signed, children, root }: TreemapCellProps) {
  if (depth === 0 || (children && children.length > 0)) return <g />;
  const nested = root !== undefined && root.depth > 0;
  const near = (a: number, b: number) => nested && Math.abs(a - b) < 0.5;
  const left = x + (near(x, root?.x ?? 0) ? 2 : 1);
  const top = y + (near(y, root?.y ?? 0) ? 2 : 1);
  const right = x + width - (near(x + width, (root?.x ?? 0) + (root?.width ?? 0)) ? 2 : 1);
  const bottom = y + height - (near(y + height, (root?.y ?? 0) + (root?.height ?? 0)) ? 2 : 1);
  const cellWidth = right - left;
  const cellHeight = bottom - top;
  if (cellWidth <= 0 || cellHeight <= 0) return <g />;

  const maxChars = Math.floor((cellWidth - 8) / CHAR_WIDTH);
  const showName = cellWidth > 36 && cellHeight > 18;
  const showValue = showName && cellHeight > 34 && signed !== undefined;
  return (
    <g>
      <rect x={left} y={top} width={cellWidth} height={cellHeight} rx={2} fill={negative ? 'var(--color-negative)' : 'var(--color-value)'} />
      {showName && (
        <text x={left + 4} y={top + 14} fill="#fff" fontSize={12} fontWeight={500} className="pointer-events-none">
          {truncate(name, maxChars)}
        </text>
      )}
      {showValue && (
        <text x={left + 4} y={top + 30} fill="#fff" fillOpacity={0.85} fontSize={11} className="pointer-events-none tabular-nums">
          {truncate(compactNumber.format(new BigNumber(signed).toNumber()), maxChars)}
        </text>
      )}
    </g>
  );
}

function QueryTreemap({ points, currency }: { points: ChartPoint[]; currency: string }) {
  const { t } = useTranslation();
  const isMobile = useMediaQuery('(max-width: 640px)');
  const { nodes, hasPositive, hasNegative } = useMemo(() => buildTreemap(points, currency), [points, currency]);
  if (nodes.length === 0) return <NothingToPlot />;

  const tooltip = ({ active, payload }: TooltipProps<number, string>) => {
    const datum = payload?.[0]?.payload as TreemapDatum | undefined;
    if (!active || !datum || datum.signed === undefined) return null;
    return <TooltipBox title={datum.account} value={formatExact(datum.signed, currency)} negative={datum.negative} />;
  };

  return (
    <div className="flex flex-col gap-2">
      <ChartContainer config={chartConfig} className="aspect-auto w-full" style={{ height: isMobile ? 280 : 360 }}>
        <Treemap data={nodes} dataKey="size" nameKey="account" type="flat" isAnimationActive={false} content={<TreemapCell />}>
          <ChartTooltip content={tooltip} isAnimationActive={false} />
        </Treemap>
      </ChartContainer>
      {hasNegative && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {hasPositive && <LegendItem color="var(--color-value)" label={t('query.chart.positive')} />}
          <LegendItem color="var(--color-negative)" label={t('query.chart.negative')} />
          <span>{t('query.chart.treemap_absolute')}</span>
        </div>
      )}
    </div>
  );
}

// ---- bar chart ----

function QueryBarChart({ points, currency }: { points: ChartPoint[]; currency: string }) {
  const { t } = useTranslation();
  const isMobile = useMediaQuery('(max-width: 640px)');
  const bars = useMemo(() => buildBars(points, currency), [points, currency]);
  if (bars.length === 0) return <NothingToPlot />;

  const shown = bars.slice(0, MAX_BARS);
  const hasNegative = shown.some((bar) => bar.value < 0);
  const hasPositive = shown.some((bar) => bar.value > 0);
  // Bars grow from zero, so the value axis always includes it: all-positive [0, max], all-negative [min, 0], mixed
  // [min, max]. The data side stays 'auto' so recharts rounds it to nice ticks.
  const valueDomain: [number | 'auto', number | 'auto'] = [hasNegative ? 'auto' : 0, hasPositive ? 'auto' : 0];
  const longestLabel = Math.max(...shown.map((bar) => (bar.label || '—').length));
  const labelWidth = Math.min(isMobile ? 104 : 200, Math.max(40, Math.ceil(longestLabel * CHAR_WIDTH) + 8));
  const labelChars = Math.floor((labelWidth - 8) / CHAR_WIDTH);

  const tooltip = ({ active, payload }: TooltipProps<number, string>) => {
    const datum = payload?.[0]?.payload as BarDatum | undefined;
    if (!active || !datum) return null;
    return <TooltipBox title={datum.label || '—'} value={formatExact(datum.signed, currency)} negative={datum.value < 0} />;
  };

  return (
    <div className="flex flex-col gap-2">
      <ChartContainer config={chartConfig} className="aspect-auto w-full" style={{ height: shown.length * BAR_HEIGHT + 40 }}>
        <BarChart data={shown} layout="vertical" margin={{ top: 4, right: 16, bottom: 0, left: 0 }}>
          <CartesianGrid horizontal={false} />
          <XAxis
            type="number"
            domain={valueDomain}
            tickFormatter={(value: number) => compactNumber.format(value)}
            tickLine={false}
            axisLine={false}
            tickMargin={4}
          />
          <YAxis
            type="category"
            dataKey="label"
            width={labelWidth}
            interval={0}
            tickLine={false}
            axisLine={false}
            tickFormatter={(label: string) => truncate(label || '—', labelChars)}
          />
          <ChartTooltip cursor={{ fill: 'hsl(var(--muted))' }} content={tooltip} isAnimationActive={false} />
          {hasNegative && hasPositive && <ReferenceLine x={0} stroke="hsl(var(--border))" />}
          <Bar dataKey="value" radius={2} maxBarSize={20} isAnimationActive={false}>
            {shown.map((bar, index) => (
              <Cell key={index} fill={bar.value < 0 ? 'var(--color-negative)' : 'var(--color-value)'} />
            ))}
          </Bar>
        </BarChart>
      </ChartContainer>
      {(hasNegative || bars.length > shown.length) && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {hasNegative && hasPositive && <LegendItem color="var(--color-value)" label={t('query.chart.positive')} />}
          {hasNegative && <LegendItem color="var(--color-negative)" label={t('query.chart.negative')} />}
          {bars.length > shown.length && <span>{t('query.chart.bars_truncated', { shown: shown.length, total: bars.length })}</span>}
        </div>
      )}
    </div>
  );
}

// ---- line chart ----

const DAY = 24 * 60 * 60 * 1000;

/** Calendar-aligned ticks (years, months or days, depending on the span) between the first and last date. */
function timeTicks(first: number, last: number, maxTicks: number): { ticks: number[]; pattern: string } {
  const spanDays = (last - first) / DAY;
  const interval = { start: first, end: last };
  let ticks: number[];
  let pattern: string;
  if (spanDays > 3 * 366) {
    ticks = eachYearOfInterval(interval).map((date) => date.getTime());
    pattern = 'yyyy';
  } else if (spanDays > 62) {
    ticks = eachMonthOfInterval(interval).map((date) => date.getTime());
    pattern = 'yyyy-MM';
  } else {
    ticks = eachDayOfInterval(interval).map((date) => date.getTime());
    pattern = 'MM-dd';
  }
  ticks = ticks.filter((tick) => tick >= first && tick <= last);
  const step = Math.ceil(ticks.length / maxTicks);
  ticks = ticks.filter((_, index) => index % step === 0);
  return { ticks: ticks.length > 0 ? ticks : [first], pattern: ticks.length > 0 ? pattern : 'yyyy-MM-dd' };
}

function QueryLineChart({ points, currency }: { points: ChartPoint[]; currency: string }) {
  const isMobile = useMediaQuery('(max-width: 640px)');
  const data = useMemo(() => buildLine(points, currency), [points, currency]);
  if (data.length === 0) return <NothingToPlot />;

  const first = data[0].time;
  const last = data[data.length - 1].time;
  // a single date gets a day of room on both sides, so the point is not drawn on the axis edge
  const domain = first === last ? [first - DAY, last + DAY] : [first, last];
  const { ticks, pattern } = timeTicks(domain[0], domain[1], isMobile ? 4 : 8);
  const values = data.map((datum) => datum.value);
  const crossesZero = Math.min(...values) < 0 && Math.max(...values) > 0;

  const tooltip = ({ active, payload }: TooltipProps<number, string>) => {
    const datum = payload?.[0]?.payload as LineDatum | undefined;
    if (!active || !datum) return null;
    return <TooltipBox title={datum.date} value={formatExact(datum.signed, currency)} />;
  };

  return (
    <ChartContainer config={chartConfig} className="aspect-auto w-full" style={{ height: isMobile ? 240 : 320 }}>
      <LineChart data={data} margin={{ top: 8, right: 16, bottom: 0, left: 0 }}>
        <CartesianGrid vertical={false} />
        <XAxis
          dataKey="time"
          type="number"
          scale="time"
          domain={domain}
          ticks={ticks}
          tickFormatter={(time: number) => format(time, pattern)}
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          minTickGap={16}
        />
        {/* a line needs no zero baseline: 'auto' fits the data range, negative or not, with rounded ticks */}
        <YAxis
          domain={['auto', 'auto']}
          tickFormatter={(value: number) => compactNumber.format(value)}
          width={56}
          tickLine={false}
          axisLine={false}
          tickMargin={4}
        />
        <ChartTooltip cursor={{ stroke: 'hsl(var(--border))' }} content={tooltip} isAnimationActive={false} />
        {crossesZero && <ReferenceLine y={0} stroke="hsl(var(--border))" />}
        <Line
          dataKey="value"
          type="linear"
          stroke="var(--color-value)"
          strokeWidth={2}
          dot={data.length <= 40 ? { r: 3, fill: 'var(--color-value)', strokeWidth: 0 } : false}
          activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--background))' }}
          isAnimationActive={false}
        />
      </LineChart>
    </ChartContainer>
  );
}

// ---- chart panel ----

function NothingToPlot() {
  const { t } = useTranslation();
  return <p className="py-8 text-center text-sm text-muted-foreground">{t('query.chart.nothing_to_plot')}</p>;
}

interface Props {
  result: QueryResult;
  kind: QueryChartKind;
  /** the ledger's operating currency, plotted by default when the result has it */
  operatingCurrency?: string;
}

export default function QueryResultChart({ result, kind, operatingCurrency }: Props) {
  const { t } = useTranslation();
  // the colour variables are also defined on the panel, for the legends rendered outside the chart containers
  const panelId = `query-chart-${useId().replace(/:/g, '')}`;
  const points = useMemo(() => collectPoints(result), [result]);
  const currencies = useMemo(() => currenciesOf(points), [points]);
  // the picked currency is kept across runs and used again whenever the new result has it
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const currency = picked !== undefined && currencies.includes(picked) ? picked : defaultCurrency(currencies, operatingCurrency);

  let chart: ReactNode;
  if (currency === undefined) chart = <NothingToPlot />;
  else if (kind === 'treemap') chart = <QueryTreemap points={points} currency={currency} />;
  else if (kind === 'bar') chart = <QueryBarChart points={points} currency={currency} />;
  else chart = <QueryLineChart points={points} currency={currency} />;

  return (
    <div data-chart={panelId} className="flex flex-col gap-3 rounded-md border p-3">
      <ChartStyle id={panelId} config={chartConfig} />
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">{t(`query.chart.${kind}`)}</span>
        {currencies.length > 1 && currency !== undefined && (
          <Select value={currency} onValueChange={setPicked}>
            <SelectTrigger className="h-8 w-auto min-w-[7rem] gap-2" aria-label={t('query.chart.currency')}>
              <span className="text-muted-foreground">{t('query.chart.currency')}</span>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {currencies.map((item) => (
                <SelectItem key={item} value={item}>
                  {item}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
        {currencies.length === 1 && currency !== NO_CURRENCY && <span className="text-sm text-muted-foreground">{currency}</span>}
      </div>
      {chart}
    </div>
  );
}
