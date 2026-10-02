// Relative imports with extensions and type-only imports keep this module runnable by `node --test` (chartData.test.ts).
import type { QueryResult } from '@/api/types';
import BigNumber from 'bignumber.js';
import { isAmount, isInventory, isPosition } from './values.ts';

/**
 * Automatic charts for query results, in the spirit of beanquery/Fava. Two-column results are charted as:
 *
 * - `(str, value)` where every label looks like an account: a treemap over the account hierarchy
 * - `(str, value)` otherwise: a bar chart
 * - `(date, value)`: a line chart over time
 *
 * and results with more columns, such as PIVOT BY results (a label column, then one column per pivot value), as:
 *
 * - `(str, value, value, ...)`: a grouped bar chart, one series per value column
 * - `(date, value, value, ...)`: a multi-series line chart over time, one series per value column
 *
 * where `value` is an inventory, position, amount, decimal or int. Positions and inventories are plotted by their
 * units, one currency at a time. Values are kept exact (BigNumber plus the decimal scale of the source strings) and only
 * converted to floats for plotting.
 */
export type QueryChartKind = 'treemap' | 'bar' | 'line' | 'grouped_bar' | 'multi_line';

const VALUE_TYPES = new Set(['int', 'decimal', 'amount', 'position', 'inventory']);
/**
 * An account name, or a root such as `Assets`: `:`-separated components without whitespace or `/`, the root starting
 * with an uppercase letter and the others with an uppercase letter, a digit or a non-Latin letter (e.g. `Assets:银行`).
 * This rejects times such as `10:30` and URLs.
 */
const ACCOUNT_PATTERN = /^\p{Lu}[^\s:/]*(?::[\p{Lu}\p{Lo}\p{N}][^\s:/]*)*$/u;
/**
 * A running balance column: JOURNAL's `balance`, or a function of it such as `cost(balance)` (JOURNAL ... AT cost).
 * Its rows are cumulative, so several rows on the same label keep the last one instead of being summed.
 */
const RUNNING_BALANCE_PATTERN = /^balance$|\(balance\)$/i;

/** The currency key of plain numbers (`int` and `decimal` columns). */
export const NO_CURRENCY = '';

/** An exact signed value with the number of decimal places of its source strings, so `1.10` stays `1.10`. */
export interface ExactValue {
  value: BigNumber;
  scale: number;
}

const ZERO: ExactValue = { value: new BigNumber(0), scale: 0 };

function plus(a: ExactValue, b: ExactValue): ExactValue {
  return { value: a.value.plus(b.value), scale: Math.max(a.scale, b.scale) };
}

/** The exact decimal string, e.g. `-1234.50`. */
export function exactString({ value, scale }: ExactValue): string {
  return value.toFixed(scale);
}

/** Signed values of one result row (or label), keyed by currency. */
export type CurrencyValues = Map<string, ExactValue>;

export interface ChartPoint {
  label: string;
  values: CurrencyValues;
}

export function detectChartKind(result: QueryResult): QueryChartKind | null {
  if (result.rows.length === 0) return null;
  if (result.columns.length > 2) return detectSeriesChartKind(result);
  if (result.columns.length !== 2) return null;
  const [label, value] = result.columns;
  if (!VALUE_TYPES.has(value.type)) return null;
  if (label.type === 'date') return 'line';
  if (label.type !== 'str') return null;
  const labels = result.rows.map((row) => row[0]).filter((cell): cell is string => typeof cell === 'string' && cell !== '');
  const looksLikeAccounts = labels.some((name) => name.includes(':')) && labels.every((name) => ACCOUNT_PATTERN.test(name));
  return looksLikeAccounts ? 'treemap' : 'bar';
}

/** A date or string label column followed by value columns only, e.g. a PIVOT BY result: one series per value column. */
function detectSeriesChartKind(result: QueryResult): QueryChartKind | null {
  const [label, ...values] = result.columns;
  if (!values.every((column) => VALUE_TYPES.has(column.type))) return null;
  if (label.type === 'date') return 'multi_line';
  if (label.type === 'str') return 'grouped_bar';
  return null;
}

function add(values: CurrencyValues, currency: string, number: unknown) {
  if (typeof number !== 'string' && typeof number !== 'number') return;
  const value = new BigNumber(number);
  if (value.isNaN()) return;
  const fraction = /^[+-]?\d*\.(\d+)$/.exec(String(number));
  const scale = fraction ? fraction[1].length : (value.decimalPlaces() ?? 0);
  values.set(currency, plus(values.get(currency) ?? ZERO, { value, scale }));
}

/** Signed values of a cell keyed by currency, or `null` for a NULL cell. An empty inventory has no values. */
function cellValues(type: string, cell: unknown): CurrencyValues | null {
  if (cell === null || cell === undefined) return null;
  const values: CurrencyValues = new Map();
  switch (type) {
    case 'int':
    case 'decimal':
      add(values, NO_CURRENCY, cell);
      break;
    case 'amount':
      if (isAmount(cell)) add(values, cell.currency, cell.number);
      break;
    case 'position':
      if (isPosition(cell)) add(values, cell.units.currency, cell.units.number);
      break;
    case 'inventory':
      if (isInventory(cell)) {
        cell.positions.filter(isPosition).forEach((position) => add(values, position.units.currency, position.units.number));
      }
      break;
    default:
      break;
  }
  return values;
}

export function isRunningBalance(columnName: string): boolean {
  return RUNNING_BALANCE_PATTERN.test(columnName.trim());
}

/** The label of a row, or `null` for a row without one (a NULL date can't be placed on a time axis). */
function labelOf(labelType: string, label: unknown): string | null {
  if (label === null || label === undefined) return labelType === 'date' ? null : '';
  return String(label);
}

/**
 * One point per distinct label of the value column `valueIndex` (the second column by default), in the order labels
 * first appear. Rows with a NULL value are skipped. Rows sharing a label are summed (flows, e.g. `SELECT date,
 * position`), except for a running balance column, where the last row of the label wins (e.g. JOURNAL's `balance`: two
 * postings on one day with balances 100 then 150 plot as 150).
 */
export function collectPoints(result: QueryResult, valueIndex = 1): ChartPoint[] {
  const labelColumn = result.columns[0];
  const valueColumn = result.columns[valueIndex];
  const keepLast = isRunningBalance(valueColumn.name);
  const points = new Map<string, CurrencyValues>();
  for (const row of result.rows) {
    const key = labelOf(labelColumn.type, row[0]);
    if (key === null) continue;
    const values = cellValues(valueColumn.type, row[valueIndex]);
    if (values === null) continue;
    const previous = points.get(key);
    if (keepLast || !previous) {
      points.set(key, values);
    } else {
      values.forEach((value, currency) => previous.set(currency, plus(previous.get(currency) ?? ZERO, value)));
    }
  }
  return Array.from(points, ([label, values]) => ({ label, values }));
}

/** Currencies with at least one non-zero value, the most frequent first. */
export function currenciesOf(points: ChartPoint[]): string[] {
  const counts = new Map<string, number>();
  points.forEach((point) =>
    point.values.forEach((value, currency) => {
      if (!value.value.isZero()) counts.set(currency, (counts.get(currency) ?? 0) + 1);
    }),
  );
  return Array.from(counts)
    .sort(([a, countA], [b, countB]) => countB - countA || a.localeCompare(b))
    .map(([currency]) => currency);
}

/** The operating currency when the result has it, otherwise the most frequent currency. */
export function defaultCurrency(currencies: string[], operatingCurrency?: string): string | undefined {
  if (operatingCurrency && currencies.includes(operatingCurrency)) return operatingCurrency;
  return currencies[0];
}

// ---- treemap ----

export interface TreemapDatum {
  /** the last account component, shown as the cell label */
  name: string;
  /** the full account name */
  account: string;
  /** leaves only: the absolute value, which sizes the cell */
  size?: number;
  /** leaves only: the exact signed value */
  signed?: string;
  negative?: boolean;
  children?: TreemapDatum[];
}

interface AccountTrie {
  name: string;
  account: string;
  own: ExactValue | null;
  children: Map<string, AccountTrie>;
}

function weightOf(node: TreemapDatum): number {
  return node.children ? node.children.reduce((sum, child) => sum + weightOf(child), 0) : (node.size ?? 0);
}

function toTreemap(trie: AccountTrie): TreemapDatum | null {
  const children = Array.from(trie.children.values())
    .map(toTreemap)
    .filter((child): child is TreemapDatum => child !== null);
  const own: TreemapDatum | null =
    trie.own && !trie.own.value.isZero()
      ? {
          name: trie.name,
          account: trie.account,
          size: trie.own.value.abs().toNumber(),
          signed: exactString(trie.own),
          negative: trie.own.value.isNegative(),
        }
      : null;
  if (children.length === 0) return own;
  // an account with both its own postings and sub-accounts shows its own value as a cell next to the sub-accounts
  if (own) children.push(own);
  children.sort((a, b) => weightOf(b) - weightOf(a));
  return { name: trie.name, account: trie.account, children };
}

export interface TreemapData {
  nodes: TreemapDatum[];
  hasPositive: boolean;
  hasNegative: boolean;
}

/**
 * Builds the account hierarchy of the values in `currency`. Cells are sized by absolute value; the sign is kept in
 * `signed`/`negative`. A common prefix shared by every account (e.g. `Expenses` when only expenses are queried) is
 * collapsed, so the top level is the first level that actually splits.
 */
export function buildTreemap(points: ChartPoint[], currency: string): TreemapData {
  const root: AccountTrie = { name: '', account: '', own: null, children: new Map() };
  let hasPositive = false;
  let hasNegative = false;
  for (const point of points) {
    const value = point.values.get(currency);
    if (!value || value.value.isZero() || point.label === '') continue;
    if (value.value.isNegative()) hasNegative = true;
    else hasPositive = true;
    let node = root;
    for (const component of point.label.split(':')) {
      const account = node.account ? `${node.account}:${component}` : component;
      let child = node.children.get(component);
      if (!child) {
        child = { name: component, account, own: null, children: new Map() };
        node.children.set(component, child);
      }
      node = child;
    }
    node.own = plus(node.own ?? ZERO, value);
  }
  let nodes = toTreemap(root)?.children ?? [];
  while (nodes.length === 1 && nodes[0].children) nodes = nodes[0].children;
  return { nodes, hasPositive, hasNegative };
}

// ---- bar chart ----

export interface BarDatum {
  label: string;
  value: number;
  signed: string;
}

/** One bar per label that has a value in `currency`, in result order. */
export function buildBars(points: ChartPoint[], currency: string): BarDatum[] {
  return points.flatMap((point) => {
    const value = point.values.get(currency);
    return value ? [{ label: point.label, value: value.value.toNumber(), signed: exactString(value) }] : [];
  });
}

// ---- line chart ----

export interface LineDatum {
  /** local midnight of `date`, in milliseconds */
  time: number;
  date: string;
  value: number;
  signed: string;
}

/** Local midnight of a `YYYY-MM-DD` date. `setFullYear` avoids `new Date(y, m, d)` mapping years 0-99 to 1900-1999. */
export function localTime(date: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) return null;
  const time = new Date(2000, 0, 1);
  time.setFullYear(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return time.getTime();
}

/** One point per date in ascending order. A date without a value in `currency` is plotted as zero. */
export function buildLine(points: ChartPoint[], currency: string): LineDatum[] {
  return points
    .flatMap((point) => {
      const time = localTime(point.label);
      if (time === null) return [];
      const value = point.values.get(currency) ?? ZERO;
      return [{ time, date: point.label, value: value.value.toNumber(), signed: exactString(value) }];
    })
    .sort((a, b) => a.time - b.time);
}

// ---- multi-series charts (PIVOT BY results) ----

/** The most series a chart draws, one per categorical colour. Further value columns are left to the table. */
export const MAX_SERIES = 8;

export interface Series {
  /** the value column name, e.g. a pivot value such as `2024` */
  name: string;
  /** the position among the value columns, which also picks the series colour */
  index: number;
  points: ChartPoint[];
}

export interface SeriesSet {
  /** the distinct labels, in the order they first appear in the rows */
  labels: string[];
  /** one series per value column, up to MAX_SERIES */
  series: Series[];
  /** the number of value columns, including those beyond MAX_SERIES */
  total: number;
}

/** One series per value column (every column after the first), each collected like a two-column result. */
export function collectSeries(result: QueryResult): SeriesSet {
  const labelType = result.columns[0].type;
  const labels = new Set<string>();
  for (const row of result.rows) {
    const label = labelOf(labelType, row[0]);
    if (label !== null) labels.add(label);
  }
  const valueColumns = result.columns.slice(1);
  const series = valueColumns.slice(0, MAX_SERIES).map((column, index) => ({ name: column.name, index, points: collectPoints(result, index + 1) }));
  return { labels: Array.from(labels), series, total: valueColumns.length };
}

/** Currencies with at least one non-zero value in any series, the most frequent first. */
export function seriesCurrencies(set: SeriesSet): string[] {
  return currenciesOf(set.series.flatMap((series) => series.points));
}

export interface SeriesDatum {
  label: string;
  /** per drawn series: the value in the currency, or `null` when the label has none */
  values: (number | null)[];
  /** per drawn series: the exact signed value, or `null` when the label has none */
  signed: (string | null)[];
}

export interface SeriesLineDatum extends SeriesDatum {
  /** local midnight of the date label, in milliseconds */
  time: number;
}

export interface SeriesChartData<T extends SeriesDatum> {
  /** the series with a value in the currency, in column order; `values` and `signed` follow this order */
  series: Series[];
  data: T[];
}

/** The series with a value in `currency`, and their values in it keyed by label. */
function seriesIn(set: SeriesSet, currency: string): { series: Series[]; byLabel: Map<string, ExactValue>[] } {
  const series = set.series.filter((item) => item.points.some((point) => point.values.has(currency)));
  const byLabel = series.map(
    (item) => new Map(item.points.flatMap((point) => (point.values.has(currency) ? [[point.label, point.values.get(currency) as ExactValue]] : []))),
  );
  return { series, byLabel };
}

/** One group of bars per label with a value in `currency`, in result order; a label without a value in a series has no bar there. */
export function buildSeriesBars(set: SeriesSet, currency: string): SeriesChartData<SeriesDatum> {
  const { series, byLabel } = seriesIn(set, currency);
  const data = set.labels.flatMap((label) => {
    const exact = byLabel.map((values) => values.get(label) ?? null);
    if (exact.every((value) => value === null)) return [];
    return [{ label, values: exact.map((value) => value?.value.toNumber() ?? null), signed: exact.map((value) => (value ? exactString(value) : null)) }];
  });
  return { series, data };
}

/** One point per date in ascending order, for every series. A date without a value in a series is plotted as zero there, as in a line chart. */
export function buildSeriesLines(set: SeriesSet, currency: string): SeriesChartData<SeriesLineDatum> {
  const { series, byLabel } = seriesIn(set, currency);
  const data = set.labels
    .flatMap((label) => {
      const time = localTime(label);
      if (time === null) return [];
      const exact = byLabel.map((values) => values.get(label) ?? ZERO);
      return [{ label, time, values: exact.map((value) => value.value.toNumber()), signed: exact.map(exactString) }];
    })
    .sort((a, b) => a.time - b.time);
  return { series, data };
}
