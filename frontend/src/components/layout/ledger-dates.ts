import { format } from 'date-fns';

/**
 * A range as the ledger dates the report endpoints take: `yyyy-MM-dd`, both inclusive. These are the days picked on the
 * calendar, so the report covers them whatever the browser's timezone (an instant would be read in the ledger's).
 */
export function ledgerDates(range: { from: Date; to: Date }) {
  return { from: format(range.from, 'yyyy-MM-dd'), to: format(range.to, 'yyyy-MM-dd') };
}
