// The ledger's "now": the default date and time of what the forms write, and the instant they ask the open accounts at.
// Type-only imports keep this module runnable by `node --test` (ledger-now.test.ts).
import type { Builtins } from '@/api/builtins';
import type { LedgerDateTime } from './ledger-datetime';

export type LedgerNowRow = Builtins['ledger.now']['row'];

/** A wall-clock instant of the ledger's timezone: the `date` and `time` parameters of the built-in queries. */
export interface LedgerInstant {
  /** `YYYY-MM-DD` */
  date: string;
  /** `HH:MM:SS` */
  time: string;
}

/** `datetime`, the ledger's wall-clock time `YYYY-MM-DDTHH:MM:SS`, as an instant. */
export function instantOf(datetime: LedgerDateTime): LedgerInstant {
  const [date, time = '00:00:00'] = datetime.split('T');
  return { date, time: time.slice(0, 8) };
}

/** An instant as the ledger's wall-clock time, `YYYY-MM-DDTHH:MM:SS`. */
export function datetimeOf({ date, time }: LedgerInstant): LedgerDateTime {
  return `${date}T${time}`;
}

/** The browser's clock as a wall-clock instant, to the second. */
export function browserInstant(at = new Date()): LedgerInstant {
  const pad = (number: number) => String(number).padStart(2, '0');
  return {
    date: `${pad(at.getFullYear()).padStart(4, '0')}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`,
    time: `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`,
  };
}

/**
 * The ledger's current instant from the row of `ledger.now`: its clock in its timezone, what `today()` and `now()` give.
 * A ledger without any account yields no row (the query reads `#accounts`); the browser's clock stands in then, which is
 * harmless because without accounts neither form can submit anything.
 */
export function ledgerNow(rows: LedgerNowRow[], fallback: () => LedgerInstant = browserInstant): LedgerInstant {
  const row = rows[0];
  return row?.date && row.time ? { date: row.date, time: row.time } : fallback();
}
