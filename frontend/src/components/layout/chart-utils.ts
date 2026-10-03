import BigNumber from 'bignumber.js';
import { parseISO } from 'date-fns';
import { max, min, sortBy } from 'lodash-es';
import { OpReturnType } from 'openapi-typescript-fetch';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { operations } from '@/api/schemas';
import { AccountType } from '@/api/types';
import { spansYears, useDateFormat } from './use-date-format';

export type StatisticGraphResponse = OpReturnType<operations['get_statistic_graph']>['data'];
export type GraphInterval = 'Day' | 'Week' | 'Month';

export interface GraphRow {
  key: string;
  label: string;
  fullLabel: string;
  /** Net worth (assets + liabilities) at the end of the bucket. */
  total: number;
  /** Income of the bucket, positive. */
  income: number;
  /** Expenses of the bucket, negative so the bars grow below the zero line. */
  expense: number;
}

/** The `:interval` of the graph's built-in queries (`report.net_worth`, `report.changes`) for a bucket size. */
export function intervalStride(interval: GraphInterval): string {
  return { Day: '1 day', Week: '1 week', Month: '1 month' }[interval];
}

/** Pick a bucket size that keeps the bar count readable: daily up to ~6 weeks, weekly up to ~6 months, then monthly. */
export function intervalForRange(from: Date, to: Date): GraphInterval {
  const days = (to.getTime() - from.getTime()) / 86_400_000;
  if (days <= 45) return 'Day';
  if (days <= 200) return 'Week';
  return 'Month';
}

export function useGraphRows(data: StatisticGraphResponse | undefined, interval: GraphInterval = 'Day'): { rows: GraphRow[]; commodity: string } {
  const fmt = useDateFormat();
  const { t } = useTranslation();
  return React.useMemo(() => {
    if (!data) return { rows: [], commodity: '' };
    const dates = sortBy(Object.keys(data.balances), (date) => parseISO(date).getTime());
    const commodity = dates.length > 0 ? data.balances[dates[dates.length - 1]].calculated.commodity : '';
    // Day / week ticks (`Sep 16`) need the year when the range crosses a year boundary.
    const multiYear = spansYears(dates.map((date) => parseISO(date)));
    const dayLabel = (day: Date) => (multiYear ? fmt.date(day) : fmt.day(day));
    const rows = dates.map((date) => {
      const day = parseISO(date);
      const changes = data.changes[date];
      return {
        key: date,
        label: interval === 'Month' ? fmt.month(day) : dayLabel(day),
        fullLabel: interval === 'Month' ? fmt.month(day) : interval === 'Week' ? t('ledger.chart.week_of', { date: fmt.date(day) }) : fmt.weekdayDate(day),
        total: new BigNumber(data.balances[date].calculated.number).toNumber(),
        income: -1 * new BigNumber(changes?.[AccountType.Income]?.calculated.number ?? '0').toNumber(),
        expense: -1 * new BigNumber(changes?.[AccountType.Expenses]?.calculated.number ?? '0').toNumber(),
      };
    });
    return { rows, commodity };
  }, [data, interval, fmt, t]);
}

/** Round tick values (steps of 1 / 2 / 5 × 10ⁿ) covering `[low, high]`. */
export function niceTicks(low: number, high: number, count = 4) {
  const span = high - low || Math.max(Math.abs(high) * 0.02, 1);
  const raw = span / count;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * pow).find((it) => it >= raw) ?? 10 * pow;
  const first = Math.floor((high === low ? low - span / 2 : low) / step) * step;
  const last = Math.ceil((high === low ? high + span / 2 : high) / step) * step;
  const ticks: number[] = [];
  for (let value = first; value <= last + step / 2; value += step) ticks.push(Number(value.toPrecision(12)));
  return { ticks, step };
}

/** Y axis with round ticks, compact labels precise enough to tell ticks apart, and a width that fits the longest label. */
export function useAxisFormatter(values: number[], count = 4) {
  const { i18n } = useTranslation();
  return React.useMemo(() => {
    const low = min(values) ?? 0;
    const high = max(values) ?? 0;
    const { ticks, step } = niceTicks(low, high, count);
    const magnitude = Math.max(...ticks.map((it) => Math.abs(it)), 1);
    const digits = Math.min(8, Math.max(2, Math.floor(Math.log10(magnitude)) - Math.floor(Math.log10(step)) + 1));
    const formatter = new Intl.NumberFormat(i18n.language, { notation: 'compact', maximumSignificantDigits: digits });
    const format = (value: number) => formatter.format(value);
    // CJK units (万, 亿) are roughly twice as wide as digits.
    const longest = Math.max(...ticks.map((it) => format(it).replace(/[^ -~]/g, '__').length), 3);
    return { format, ticks, domain: [ticks[0], ticks[ticks.length - 1]] as [number, number], width: Math.ceil(longest * 7 + 10) };
  }, [values, count, i18n.language]);
}
