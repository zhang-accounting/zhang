// The report's figures from the rows of the built-in queries `report.*`, by the rules the server's report endpoints
// followed (#479). Type-only imports keep this module runnable by `node --test` (report.test.ts).
import BigNumber from 'bignumber.js';
import type { Builtins } from '@/api/builtins';
import type { QueryAmount, QueryInventory } from '@/api/types';

export type NetWorthRow = Builtins['report.net_worth']['row'];
export type LiabilitiesRow = Builtins['report.liabilities']['row'];
export type FlowsRow = Builtins['report.flows']['row'];
export type TransactionCountRow = Builtins['report.transaction_count']['row'];
export type AccountTotalsRow = Builtins['report.account_totals']['row'];
export type TopPostingsRow = Builtins['report.top_postings']['row'];

/** An amount as the pages render it. */
export interface ReportAmount {
  number: string;
  commodity: string;
}

/**
 * A figure of the report: `calculated`, its value in the operating currency, and `detail`, its units per currency (what the
 * query could not convert stays in its own currency there, and is left out of `calculated`).
 */
export interface CalculatedAmount {
  calculated: ReportAmount;
  detail: Record<string, string>;
}

/** The summary of a range: the net worth and the liabilities at its end, and its income, expenses and transactions. */
export interface ReportSummary {
  balance: CalculatedAmount;
  liability: CalculatedAmount;
  income: CalculatedAmount;
  expense: CalculatedAmount;
  transaction_number: number;
}

/** What an account of a type changed by in the range. */
export interface RankItem {
  account: string;
  amount: CalculatedAmount;
}

/** One of the largest postings of a type in the range, as the account journal shows a posting. */
export interface TopPosting {
  datetime: string;
  timestamp: number;
  account: string;
  trx_id: string;
  payee: string | null;
  narration: string | null;
  inferred_unit: ReportAmount;
  /** the balance of the account in the posting's commodity right after it */
  account_after: ReportAmount;
}

/** The rank of a type in a range: every account's total, and the ten largest postings by value. */
export interface ReportRank {
  detail: RankItem[];
  top_transactions: TopPosting[];
}

/** A plain decimal string of a sum, without an exponent. */
function plain(number: BigNumber): string {
  return number.toFixed();
}

/**
 * The units of an inventory per currency: its positions added up by currency (lots of a currency held at different
 * costs merged), without the currencies that add up to zero, as an inventory never holds a zero. A single position of a
 * currency keeps its number as written.
 */
function unitsByCurrency(inventory: QueryInventory | null): Record<string, string> {
  const sums = new Map<string, { number: BigNumber; text: string; count: number }>();
  for (const position of inventory?.positions ?? []) {
    const { currency, number } = position.units;
    const current = sums.get(currency);
    sums.set(currency, { number: (current?.number ?? new BigNumber(0)).plus(number), text: number, count: (current?.count ?? 0) + 1 });
  }
  const units: Record<string, string> = {};
  for (const [currency, { number, text, count }] of sums) {
    if (number.isZero()) continue;
    units[currency] = count === 1 ? text : plain(number);
  }
  return units;
}

/**
 * The figure of a `units` and a `value` cell: `detail` is `units` per currency, `calculated` the `currency` part of
 * `value`, which the query computed from the units at the prices of the range's end.
 */
export function calculatedAmount(units: QueryInventory | null, value: QueryInventory | null, currency: string): CalculatedAmount {
  const converted = (value?.positions ?? []).filter((position) => position.units.currency === currency);
  const total =
    converted.length === 1 ? converted[0].units.number : plain(converted.reduce((sum, position) => sum.plus(position.units.number), new BigNumber(0)));
  return { calculated: { number: total, commodity: currency }, detail: unitsByCurrency(units) };
}

/** The figure of the first row of an aggregate query without groups; nothing when it matched no postings. */
function singleFigure(row: { units: QueryInventory | null; value: QueryInventory | null } | undefined, currency: string): CalculatedAmount {
  return calculatedAmount(row?.units ?? null, row?.value ?? null, currency);
}

/**
 * The summary of a range from the rows of `report.net_worth` and `report.liabilities` (at the range's end),
 * `report.flows` (by account type) and `report.transaction_count`, every figure in the operating `currency`.
 */
export function reportSummary(
  netWorth: NetWorthRow[],
  liabilities: LiabilitiesRow[],
  flows: FlowsRow[],
  count: TransactionCountRow[],
  currency: string,
): ReportSummary {
  const flow = (type: string) =>
    singleFigure(
      flows.find((row) => row.type === type),
      currency,
    );
  return {
    balance: singleFigure(netWorth[0], currency),
    liability: singleFigure(liabilities[0], currency),
    income: flow('Income'),
    expense: flow('Expenses'),
    // an aggregate query without rows has no row, not a zero
    transaction_number: count[0]?.transactions ?? 0,
  };
}

/** An amount cell as the pages render it. */
function amount(cell: QueryAmount): ReportAmount {
  return { number: cell.number, commodity: cell.currency };
}

/**
 * The rank of a type in a range from the rows of `report.account_totals` (every account of the type with postings in the
 * range) and `report.top_postings` (its ten largest postings by value, largest first), in the operating `currency`. A
 * row without an account, or a posting the query could not describe in full, is left out, as before.
 */
export function reportRank(totals: AccountTotalsRow[], top: TopPostingsRow[], currency: string): ReportRank {
  const detail = totals.flatMap((row) => (row.account ? [{ account: row.account, amount: calculatedAmount(row.units, row.value, currency) }] : []));
  const top_transactions = top.flatMap((row) => {
    if (row.date == null || row.timestamp == null || row.account == null || row.id == null || row.units == null || row.account_balance == null) return [];
    return [
      {
        datetime: `${row.date}T${row.time ?? '00:00:00'}`,
        timestamp: row.timestamp,
        account: row.account,
        trx_id: row.id,
        payee: row.payee,
        narration: row.narration,
        inferred_unit: amount(row.units),
        account_after: amount(row.account_balance),
      },
    ];
  });
  return { detail, top_transactions };
}
