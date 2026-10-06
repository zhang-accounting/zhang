import { endOfDay, endOfMonth, isBefore, startOfDay, startOfMonth, subDays } from 'date-fns';
import { retrieveJournals } from '@/api/requests';
import { JournalItem } from '@/api/types';
import { useLedgerQuery } from '@/states/ledger';

/** Latest journals of the whole ledger (independent of the Journals page filters). */
export function useRecentJournals(size = 6) {
  const { value, loading, error } = useLedgerQuery(() => retrieveJournals({ page: 1, size, keyword: '', tags: [], links: [] }), [size]);
  return { records: (value?.records ?? []) as JournalItem[], total: value?.total_count ?? 0, loading: loading || (!value && !error), error };
}

/**
 * Date the dashboard / report should be anchored to: today, unless the ledger has had no activity for more than 30 days
 * (e.g. an imported or example ledger), in which case it is the day of the latest journal so the charts show real data.
 */
export function activityAnchor(latest: JournalItem | undefined, now = new Date()): { anchor: Date; stale: boolean } {
  if (!latest) return { anchor: now, stale: false };
  const latestDate = new Date(latest.datetime);
  const stale = isBefore(latestDate, subDays(now, 30));
  return { anchor: stale ? latestDate : now, stale };
}

/** Trailing 30-day window ending on `anchor`. */
export function trailingMonth(anchor: Date) {
  return { from: startOfDay(subDays(anchor, 30)), to: endOfDay(anchor) };
}

/** Calendar month containing `anchor`. */
export function monthOf(anchor: Date) {
  return { from: startOfMonth(anchor), to: endOfMonth(anchor) };
}
