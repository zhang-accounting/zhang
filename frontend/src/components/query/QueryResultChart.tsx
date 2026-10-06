import BigNumber from 'bignumber.js';
import { eachDayOfInterval, eachMonthOfInterval, eachYearOfInterval, format } from 'date-fns';
import { ReactNode, useId, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Bar, BarChart, CartesianGrid, Cell, Line, LineChart, ReferenceLine, TooltipContentProps, Treemap, XAxis, YAxis } from 'recharts';
import type { NameType, ValueType } from 'recharts/types/component/DefaultTooltipContent';
import { QueryResult } from '@/api/types';
import { useIsMobile } from '@/components/layout';
import {
  buildSeriesBars,
  buildSeriesLines,
  buildTreemap,
  ChartPoint,
  collectPoints,
  collectSeries,
  currenciesOf,
  defaultCurrency,
  MAX_SERIES,
  NO_CURRENCY,
  QueryChartKind,
  Series,
  SeriesDatum,
  SeriesLineDatum,
  SeriesSet,
  seriesCurrencies,
  TreemapDatum,
} from '@/components/query/chartData';
import { formatDecimal } from '@/components/query/values';
import { ChartConfig, ChartContainer, ChartStyle, ChartTooltip } from '@/components/ui/chart';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';

// chart-1 plots values; `negative` marks negative values in treemaps and bar charts, where the area/length is the
// absolute value or the sign would otherwise be easy to miss. Treemap labels use `--background` (white-ish on the light
// cells, near-black on the dark-theme cells), the most readable choice on both fills.
// Multi-series charts use chart-1..5 in order, one per value column (MAX_SERIES of them): a series keeps its colour when
// another one has nothing to plot in the picked currency.
const chartConfig = {
  value: { color: 'var(--chart-1)' },
  negative: { color: 'var(--negative)' },
  ...Object.fromEntries(Array.from({ length: MAX_SERIES }, (_, index) => [`series${index}`, { color: `var(--chart-${index + 1})` }])),
} satisfies ChartConfig;

const seriesColor = (series: Series) => `var(--color-series${series.index})`;

type ChartTooltipProps = TooltipContentProps<ValueType, NameType>;

const MAX_BARS = 50;
const BAR_HEIGHT = 28;
/** Average glyph width of the 12px chart labels, used when they cannot be measured. */
const CHAR_WIDTH = 6.5;

/** Compact axis and cell labels (`44.5K`, `44.5万`) in the app language, as `Amount` and the ledger charts write them. */
function useCompactNumber() {
  const { i18n } = useTranslation();
  return useMemo(() => new Intl.NumberFormat(i18n.language, { notation: 'compact', maximumFractionDigits: 1 }), [i18n.language]);
}

/** The exact signed value as the table shows it, trailing zeros included. */
function formatExact(signed: string, currency: string): string {
  const number = formatDecimal(signed);
  return currency === NO_CURRENCY ? number : `${number} ${currency}`;
}

let measureContext: CanvasRenderingContext2D | null | undefined;
let measureFontFamily = 'sans-serif';

/**
 * Rendered width of a 12px chart label in the app font. Measured on a canvas, so wide glyphs (CJK, capitals) are not
 * underestimated; falls back to the average glyph width when no canvas is available.
 */
function textWidth(text: string, fontWeight = 400): number {
  if (measureContext === undefined) {
    measureContext = typeof document === 'undefined' ? null : document.createElement('canvas').getContext('2d');
    if (measureContext) measureFontFamily = getComputedStyle(document.body).fontFamily;
  }
  if (!measureContext) return text.length * CHAR_WIDTH;
  measureContext.font = `${fontWeight} 12px ${measureFontFamily}`;
  return measureContext.measureText(text).width;
}

/** `text`, shortened with an ellipsis to fit into `maxWidth` pixels (empty when not even one character fits). */
function truncate(text: string, maxWidth: number, fontWeight = 400): string {
  if (textWidth(text, fontWeight) <= maxWidth) return text;
  let end = text.length - 1;
  while (end > 0 && textWidth(`${text.slice(0, end)}…`, fontWeight) > maxWidth) end -= 1;
  return end > 0 ? `${text.slice(0, end).trimEnd()}…` : '';
}

/**
 * Axes of the horizontal bar charts. Bars grow from zero, so the value axis always includes it: all-positive [0, max],
 * all-negative [min, 0], mixed [min, max]; the data side stays 'auto' so recharts rounds it to nice ticks. The label axis
 * fits the longest label: it draws the tick text 8px (tick size + margin) left of the bars, `labelSpace` is the rest.
 */
function barAxes(bars: { label: string }[], hasNegative: boolean, hasPositive: boolean, isMobile: boolean) {
  const valueDomain: [number | 'auto', number | 'auto'] = [hasNegative ? 'auto' : 0, hasPositive ? 'auto' : 0];
  const longestLabel = Math.max(...bars.map((bar) => textWidth(bar.label || '—')));
  const labelWidth = Math.min(isMobile ? 112 : 200, Math.max(40, Math.ceil(longestLabel) + 12));
  return { valueDomain, labelWidth, labelSpace: labelWidth - 12 };
}

function TooltipBox({ title, value, negative }: { title: string; value: string; negative?: boolean }) {
  return (
    <div className="grid max-w-xs gap-1 rounded-lg bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-xl ring-1 ring-foreground/10">
      <div className="font-medium break-all">{title}</div>
      <div className="flex items-center gap-1.5">
        <span className="size-2.5 shrink-0 rounded-[2px]" style={{ backgroundColor: negative ? 'var(--color-negative)' : 'var(--color-value)' }} />
        <span className="font-mono text-foreground tabular-nums">{value}</span>
      </div>
    </div>
  );
}

function LegendItem({ color, label }: { color: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className="size-2.5 rounded-[2px]" style={{ backgroundColor: color }} />
      {label}
    </span>
  );
}

/**
 * A tooltip listing the value of every series that has one, in series order. A single series shows just its value, in
 * the negative colour when `markNegative` is set and the value is below zero (as the bar is drawn).
 */
function SeriesTooltipBox({
  title,
  series,
  datum,
  currency,
  markNegative = false,
}: {
  title: string;
  series: Series[];
  datum: SeriesDatum;
  currency: string;
  markNegative?: boolean;
}) {
  if (series.length === 1) {
    const signed = datum.signed[0];
    if (signed === null) return null;
    return <TooltipBox title={title} value={formatExact(signed, currency)} negative={markNegative && (datum.values[0] ?? 0) < 0} />;
  }
  return (
    <div className="grid max-w-xs min-w-32 gap-1 rounded-lg bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-xl ring-1 ring-foreground/10">
      <div className="font-medium break-all">{title}</div>
      {series.map((item, index) => {
        const signed = datum.signed[index];
        if (signed === null) return null;
        return (
          <div key={item.index} className="flex items-center gap-1.5">
            <span className="size-2.5 shrink-0 rounded-[2px]" style={{ backgroundColor: seriesColor(item) }} />
            <span className="truncate text-muted-foreground">{item.name || '—'}</span>
            <span className="ml-auto pl-3 font-mono text-foreground tabular-nums">{formatExact(signed, currency)}</span>
          </div>
        );
      })}
    </div>
  );
}

/** The legend of a multi-series chart (two series or more), or the signs of a one-series bar chart, plus the notes on what was left out. */
function SeriesLegend({ series, notes, signs }: { series: Series[]; notes: (string | false)[]; signs?: { hasPositive: boolean } }) {
  const { t } = useTranslation();
  const shownNotes = notes.filter((note): note is string => note !== false);
  if (series.length < 2 && !signs && shownNotes.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      {series.length >= 2 && series.map((item) => <LegendItem key={item.index} color={seriesColor(item)} label={item.name || '—'} />)}
      {signs?.hasPositive && <LegendItem color="var(--color-value)" label={t('query.chart.positive')} />}
      {signs && <LegendItem color="var(--color-negative)" label={t('query.chart.negative')} />}
      {shownNotes.map((note) => (
        <span key={note}>{note}</span>
      ))}
    </div>
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
  compactNumber: Intl.NumberFormat;
}

/**
 * Draws a leaf cell. Group nodes are not drawn: their leaves cover them. Leaves are inset to leave a 2px gap, and
 * twice that along the border of their parent account, so sibling groups read as groups.
 */
function TreemapCell({ x = 0, y = 0, width = 0, height = 0, depth = 0, name = '', negative, signed, children, root, compactNumber }: TreemapCellProps) {
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

  const maxWidth = cellWidth - 8;
  const showName = cellWidth > 36 && cellHeight > 18;
  const showValue = showName && cellHeight > 34 && signed !== undefined;
  return (
    <g>
      <rect x={left} y={top} width={cellWidth} height={cellHeight} rx={2} fill={negative ? 'var(--color-negative)' : 'var(--color-value)'} />
      {showName && (
        <text x={left + 4} y={top + 14} fill="var(--background)" fontSize={12} fontWeight={500} className="pointer-events-none">
          {truncate(name, maxWidth, 500)}
        </text>
      )}
      {showValue && (
        <text x={left + 4} y={top + 30} fill="var(--background)" fillOpacity={0.85} fontSize={11} className="pointer-events-none tabular-nums">
          {truncate(compactNumber.format(new BigNumber(signed).toNumber()), maxWidth)}
        </text>
      )}
    </g>
  );
}

function QueryTreemap({ points, currency }: { points: ChartPoint[]; currency: string }) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const compactNumber = useCompactNumber();
  const { nodes, hasPositive, hasNegative } = useMemo(() => buildTreemap(points, currency), [points, currency]);
  if (nodes.length === 0) return <NothingToPlot />;

  const tooltip = ({ active, payload }: ChartTooltipProps) => {
    const datum = payload?.[0]?.payload as TreemapDatum | undefined;
    if (!active || !datum || datum.signed === undefined) return null;
    return <TooltipBox title={datum.account} value={formatExact(datum.signed, currency)} negative={datum.negative} />;
  };

  return (
    <div className="flex flex-col gap-2">
      <ChartContainer config={chartConfig} className="aspect-auto w-full" style={{ height: isMobile ? 280 : 360 }}>
        <Treemap data={nodes} dataKey="size" nameKey="account" type="flat" isAnimationActive={false} content={<TreemapCell compactNumber={compactNumber} />}>
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

// ---- time axis ----

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

/** The time axis of dates sorted in ascending order. A single date gets a day of room on both sides, so its point is not drawn on the axis edge. */
function timeAxis(first: number, last: number, isMobile: boolean) {
  const domain = first === last ? [first - DAY, last + DAY] : [first, last];
  return { domain, ...timeTicks(domain[0], domain[1], isMobile ? 4 : 8) };
}

// ---- bar chart (one series per value column; grouped bars for several) ----

function QueryBarChart({ set, currency }: { set: SeriesSet; currency: string }) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const compactNumber = useCompactNumber();
  const { series, data } = useMemo(() => buildSeriesBars(set, currency), [set, currency]);
  if (series.length === 0 || data.length === 0) return <NothingToPlot />;

  const shown = data.slice(0, MAX_BARS);
  const values = shown.flatMap((datum) => datum.values.filter((value): value is number => value !== null));
  const hasNegative = values.some((value) => value < 0);
  const hasPositive = values.some((value) => value > 0);
  const { valueDomain, labelWidth, labelSpace } = barAxes(shown, hasNegative, hasPositive, isMobile);
  // thinner bars for more series, with a 2px gap between the bars of a group
  const barSize = series.length > 3 ? 7 : 10;
  const groupHeight = Math.max(BAR_HEIGHT, series.length * (barSize + 2) + 12);

  const tooltip = ({ active, payload }: ChartTooltipProps) => {
    const datum = payload?.[0]?.payload as SeriesDatum | undefined;
    if (!active || !datum) return null;
    return <SeriesTooltipBox title={datum.label || '—'} series={series} datum={datum} currency={currency} markNegative />;
  };

  return (
    <div className="flex flex-col gap-2">
      <ChartContainer config={chartConfig} className="aspect-auto w-full" style={{ height: shown.length * groupHeight + 40 }}>
        <BarChart data={shown} layout="vertical" barGap={2} margin={{ top: 4, right: 16, bottom: 0, left: 0 }}>
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
            // non-breaking spaces: recharts would wrap the (already fitted) label onto two lines
            tickFormatter={(label: string) => truncate(label || '—', labelSpace).replace(/ /g, '\u00a0')}
          />
          <ChartTooltip cursor={{ fill: 'var(--muted)', opacity: 0.6 }} content={tooltip} isAnimationActive={false} />
          {hasNegative && hasPositive && <ReferenceLine x={0} stroke="var(--border)" />}
          {series.map((item, index) => (
            <Bar
              key={item.index}
              dataKey={(datum: SeriesDatum) => datum.values[index]}
              name={item.name}
              fill={seriesColor(item)}
              barSize={series.length > 1 ? barSize : undefined}
              maxBarSize={20}
              radius={2}
              isAnimationActive={false}
            >
              {/* one series: a bar's colour carries its sign, as in the treemap */}
              {series.length === 1 &&
                shown.map((datum, row) => <Cell key={row} fill={(datum.values[0] ?? 0) < 0 ? 'var(--color-negative)' : seriesColor(item)} />)}
            </Bar>
          ))}
        </BarChart>
      </ChartContainer>
      <SeriesLegend
        series={series}
        notes={[data.length > shown.length && t('query.chart.bars_truncated', { shown: shown.length, total: data.length })]}
        signs={series.length === 1 && hasNegative ? { hasPositive } : undefined}
      />
    </div>
  );
}

// ---- line chart (one series per value column) ----

function QueryLineChart({ set, currency }: { set: SeriesSet; currency: string }) {
  const isMobile = useIsMobile();
  const compactNumber = useCompactNumber();
  const { series, data } = useMemo(() => buildSeriesLines(set, currency), [set, currency]);
  if (series.length === 0 || data.length === 0) return <NothingToPlot />;

  const { domain, ticks, pattern } = timeAxis(data[0].time, data[data.length - 1].time, isMobile);
  const values = data.flatMap((datum) => datum.values.filter((value): value is number => value !== null));
  const crossesZero = Math.min(...values) < 0 && Math.max(...values) > 0;

  const tooltip = ({ active, payload }: ChartTooltipProps) => {
    const datum = payload?.[0]?.payload as SeriesLineDatum | undefined;
    if (!active || !datum) return null;
    return <SeriesTooltipBox title={datum.label} series={series} datum={datum} currency={currency} />;
  };

  return (
    <div className="flex flex-col gap-2">
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
          <ChartTooltip cursor={{ stroke: 'var(--border)' }} content={tooltip} isAnimationActive={false} />
          {crossesZero && <ReferenceLine y={0} stroke="var(--border)" />}
          {series.map((item, index) => (
            <Line
              key={item.index}
              dataKey={(datum: SeriesLineDatum) => datum.values[index]}
              name={item.name}
              type="linear"
              stroke={seriesColor(item)}
              strokeWidth={2}
              dot={data.length <= 40 ? { r: 3, fill: seriesColor(item), strokeWidth: 0 } : false}
              activeDot={{ r: 4, strokeWidth: 2, stroke: 'var(--background)' }}
              isAnimationActive={false}
            />
          ))}
        </LineChart>
      </ChartContainer>
      <SeriesLegend series={series} notes={[]} />
    </div>
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
  /** The currency the user picked; owned by the parent, so it survives new runs and table/chart switches. */
  pickedCurrency?: string;
  onPickCurrency: (currency: string) => void;
}

export default function QueryResultChart({ result, kind, operatingCurrency, pickedCurrency: picked, onPickCurrency: setPicked }: Props) {
  const { t } = useTranslation();
  // the colour variables are also defined on the panel, for the legends rendered outside the chart containers
  const panelId = `query-chart-${useId().replace(/:/g, '')}`;
  const set = useMemo(() => (kind === 'treemap' ? null : collectSeries(result)), [result, kind]);
  const points = useMemo(() => (kind === 'treemap' ? collectPoints(result) : []), [result, kind]);
  const currencies = useMemo(() => (set ? seriesCurrencies(set) : currenciesOf(points)), [set, points]);
  // the picked currency is kept across runs and used again whenever the new result has it
  const currency = picked !== undefined && currencies.includes(picked) ? picked : defaultCurrency(currencies, operatingCurrency);

  let chart: ReactNode;
  if (currency === undefined) chart = <NothingToPlot />;
  else if (!set) chart = <QueryTreemap points={points} currency={currency} />;
  else if (kind === 'bar' || kind === 'grouped_bar') chart = <QueryBarChart set={set} currency={currency} />;
  else chart = <QueryLineChart set={set} currency={currency} />;

  return (
    <div data-chart={panelId} className="flex min-w-0 flex-col gap-3 p-4">
      <ChartStyle id={panelId} config={chartConfig} />
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">{t(`query.chart.${kind}`)}</span>
        {currencies.length > 1 && currency !== undefined && (
          <Select
            items={currencies.map((item) => ({ value: item, label: item }))}
            value={currency}
            onValueChange={(value) => value !== null && setPicked(value)}
          >
            <SelectTrigger className="h-10 min-w-28 gap-2 md:h-8" aria-label={t('query.chart.currency')}>
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
      {set && set.total > set.series.length && (
        <p className="text-xs text-muted-foreground">{t('query.chart.series_truncated', { shown: set.series.length, total: set.total })}</p>
      )}
    </div>
  );
}
