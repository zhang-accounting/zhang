import { QueryResult } from '@/api/types';
import { isAmount, isInventory, isPosition } from '@/components/query/values';
import BigNumber from 'bignumber.js';

/**
 * Automatic charts for query results, in the spirit of beanquery/Fava. Only two-column results are charted:
 *
 * - `(str, value)` where every label looks like an account: a treemap over the account hierarchy
 * - `(str, value)` otherwise: a bar chart
 * - `(date, value)`: a line chart over time
 *
 * where `value` is an inventory, position, amount, decimal or int. Positions and inventories are plotted by their
 * units, one currency at a time. Values are summed exactly (BigNumber) and only converted to floats for plotting.
 */
export type QueryChartKind = 'treemap' | 'bar' | 'line';

const VALUE_TYPES = new Set(['int', 'decimal', 'amount', 'position', 'inventory']);
/** `:`-separated components without whitespace, e.g. `Assets:Bank:Checking` or a root such as `Assets`. */
const ACCOUNT_PATTERN = /^[^\s:]+(?::[^\s:]+)*$/;

/** The currency key of plain numbers (`int` and `decimal` columns). */
export const NO_CURRENCY = '';

const ZERO = new BigNumber(0);

/** Signed values of one result row (or label), keyed by currency. */
export type CurrencyValues = Map<string, BigNumber>;

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
  values.set(currency, (values.get(currency) ?? ZERO).plus(value));
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

/** One point per distinct label, in result order. Rows sharing a label are summed; rows with a NULL value are skipped. */
export function collectPoints(result: QueryResult): ChartPoint[] {
  const [labelColumn, valueColumn] = result.columns;
  const points = new Map<string, CurrencyValues>();
  for (const row of result.rows) {
    const label = row[0];
    if ((label === null || label === undefined) && labelColumn.type === 'date') continue;
    const values = cellValues(valueColumn.type, row[1]);
    if (values === null) continue;
    const key = label === null || label === undefined ? '' : String(label);
    const merged = points.get(key) ?? new Map<string, BigNumber>();
    values.forEach((value, currency) => merged.set(currency, (merged.get(currency) ?? ZERO).plus(value)));
    points.set(key, merged);
  }
  return Array.from(points, ([label, values]) => ({ label, values }));
}

/** Currencies with at least one non-zero value, the most frequent first. */
export function currenciesOf(points: ChartPoint[]): string[] {
  const counts = new Map<string, number>();
  points.forEach((point) =>
    point.values.forEach((value, currency) => {
      if (!value.isZero()) counts.set(currency, (counts.get(currency) ?? 0) + 1);
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
  own: BigNumber | null;
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
    trie.own && !trie.own.isZero()
      ? { name: trie.name, account: trie.account, size: trie.own.abs().toNumber(), signed: trie.own.toFixed(), negative: trie.own.isNegative() }
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
    if (!value || value.isZero() || point.label === '') continue;
    if (value.isNegative()) hasNegative = true;
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
    node.own = (node.own ?? ZERO).plus(value);
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
    return value ? [{ label: point.label, value: value.toNumber(), signed: value.toFixed() }] : [];
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

function localTime(date: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  return match ? new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3])).getTime() : null;
}

/** One point per date in ascending order. A date without a value in `currency` is plotted as zero. */
export function buildLine(points: ChartPoint[], currency: string): LineDatum[] {
  return points
    .flatMap((point) => {
      const time = localTime(point.label);
      if (time === null) return [];
      const value = point.values.get(currency) ?? ZERO;
      return [{ time, date: point.label, value: value.toNumber(), signed: value.toFixed() }];
    })
    .sort((a, b) => a.time - b.time);
}
