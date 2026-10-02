// Relative imports with extensions and type-only imports keep this module runnable by `node --test` (chartData.test.ts).
import type { QueryResult } from '@/api/types';
import BigNumber from 'bignumber.js';
import { isAmount, isInventory, isPosition } from './values.ts';

/**
 * Automatic charts for query results, in the spirit of beanquery/Fava. Only two-column results are charted:
 *
 * - `(str, value)` where every label looks like an account: a treemap over the account hierarchy
 * - `(str, value)` otherwise: a bar chart
 * - `(date, value)`: a line chart over time
 *
 * where `value` is an inventory, position, amount, decimal or int. Positions and inventories are plotted by their
 * units, one currency at a time. Values are kept exact (BigNumber plus the decimal scale of the source strings) and only
 * converted to floats for plotting.
 */
export type QueryChartKind = 'treemap' | 'bar' | 'line';

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
  if (result.columns.length !== 2 || result.rows.length === 0) return null;
  const [label, value] = result.columns;
  if (!VALUE_TYPES.has(value.type)) return null;
  if (label.type === 'date') return 'line';
  if (label.type !== 'str') return null;
  const labels = result.rows.map((row) => row[0]).filter((cell): cell is string => typeof cell === 'string' && cell !== '');
  const looksLikeAccounts = labels.some((name) => name.includes(':')) && labels.every((name) => ACCOUNT_PATTERN.test(name));
  return looksLikeAccounts ? 'treemap' : 'bar';
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

/**
 * One point per distinct label, in the order labels first appear. Rows with a NULL value are skipped. Rows sharing a
 * label are summed (flows, e.g. `SELECT date, position`), except for a running balance column, where the last row of
 * the label wins (e.g. JOURNAL's `balance`: two postings on one day with balances 100 then 150 plot as 150).
 */
export function collectPoints(result: QueryResult): ChartPoint[] {
  const [labelColumn, valueColumn] = result.columns;
  const keepLast = isRunningBalance(valueColumn.name);
  const points = new Map<string, CurrencyValues>();
  for (const row of result.rows) {
    const label = row[0];
    if ((label === null || label === undefined) && labelColumn.type === 'date') continue;
    const values = cellValues(valueColumn.type, row[1]);
    if (values === null) continue;
    const key = label === null || label === undefined ? '' : String(label);
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
