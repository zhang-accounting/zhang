// The totals of a month's budgets, by one rule for every page that shows them. Type-only imports keep this module runnable by
// `node --test` (month-totals.test.ts).
import BigNumber from 'bignumber.js';
import type { BudgetListItem } from '@/api/types';

export interface AmountLike {
  number: string;
  commodity: string;
}

/** An amount of one commodity, summed. */
export interface CommodityTotal {
  commodity: string;
  number: BigNumber;
}

/** Sum amounts per commodity, keeping the first-seen commodity order. */
export function sumByCommodity(amounts: AmountLike[]): CommodityTotal[] {
  const totals = new Map<string, BigNumber>();
  amounts.forEach((amount) => {
    totals.set(amount.commodity, (totals.get(amount.commodity) ?? new BigNumber(0)).plus(new BigNumber(amount.number)));
  });
  return Array.from(totals.entries()).map(([commodity, number]) => ({ commodity, number }));
}

type MonthBudget = Pick<BudgetListItem, 'closed' | 'assigned_amount' | 'activity_amount' | 'available_amount'>;

/**
 * Whether a budget counts in its month's totals. A closed budget keeps its rows, but counts only in the months before its close:
 * `closed` is whether it was closed in or before the month, as the engine's `#budgets` tells the budgets open at the time.
 */
export function countsInMonth(budget: Pick<BudgetListItem, 'closed'>): boolean {
  return !budget.closed;
}

/** What a month's budgets assigned, spent and have left, per commodity. */
export interface MonthTotals {
  assigned: CommodityTotal[];
  activity: CommodityTotal[];
  available: CommodityTotal[];
}

/**
 * The totals of a month's budgets, per commodity in the order they first appear: those of the budgets that count in the month
 * (`countsInMonth`), whatever a page shows of them. The Home card, the Budgets page and its categories add up by this rule.
 */
export function monthTotals(budgets: MonthBudget[]): MonthTotals {
  const counted = budgets.filter(countsInMonth);
  return {
    assigned: sumByCommodity(counted.map((budget) => budget.assigned_amount)),
    activity: sumByCommodity(counted.map((budget) => budget.activity_amount)),
    available: sumByCommodity(counted.map((budget) => budget.available_amount)),
  };
}

/** The assigned total of the first commodity and the activity in that commodity, for a usage bar. */
export function primaryFigures(totals: MonthTotals): { assigned?: CommodityTotal; activity?: CommodityTotal } {
  const assigned = totals.assigned[0];
  return { assigned, activity: totals.activity.find((it) => it.commodity === assigned?.commodity) ?? totals.activity[0] };
}
