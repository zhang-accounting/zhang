import BigNumber from 'bignumber.js';
import { cn } from '@/lib/utils';

export interface BudgetUsage {
  /** 0..100, for the progress bar. */
  percent: number;
  /** Uncapped percentage, for the label. */
  label: string;
  /** Activity exceeded the assigned amount. */
  over: boolean;
}

/** How much of the assigned amount has been used (activity / assigned). */
export function budgetUsage(activity: string | BigNumber, assigned: string | BigNumber): BudgetUsage {
  const used = new BigNumber(activity);
  const total = new BigNumber(assigned);
  if (total.isZero() || total.isNaN()) {
    const over = used.gt(0);
    return { percent: over ? 100 : 0, label: over ? '—' : '0%', over };
  }
  const raw = used.div(total).multipliedBy(100);
  const percent = BigNumber.minimum(BigNumber.maximum(raw, 0), 100).toNumber();
  return { percent, label: `${raw.decimalPlaces(raw.abs().lt(10) ? 1 : 0).toFormat()}%`, over: used.gt(total) };
}

export { sumByCommodity, type AmountLike } from './month-totals';

/** Reads `?year=2026&month=10` (month is 1-based); falls back to the current month. */
export function monthFromSearchParams(params: URLSearchParams): Date {
  const year = Number(params.get('year'));
  const month = Number(params.get('month'));
  if (Number.isInteger(year) && Number.isInteger(month) && year > 1900 && month >= 1 && month <= 12) {
    return new Date(year, month - 1, 1);
  }
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), 1);
}

export function monthSearchParams(date: Date) {
  return { year: String(date.getFullYear()), month: String(date.getMonth() + 1) };
}

/** Progress classes: blue while within budget, destructive once activity exceeds the assigned amount. */
export function usageProgressClass(over: boolean) {
  return cn('gap-0', over && '[&_[data-slot=progress-indicator]]:bg-destructive');
}
