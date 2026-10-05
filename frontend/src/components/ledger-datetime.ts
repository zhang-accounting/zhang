// The date and time of a transaction in the form: the ledger's wall-clock time, kept as the text the API reads and writes
// (`2024-01-02T07:00:00`), never as an instant of the browser's timezone. Runnable by `node --test` (ledger-datetime.test.ts).

/** `2024-01-02T07:00:00`: a wall-clock time of the ledger's timezone, as every `datetime` of the API. */
export type LedgerDateTime = string;

const WALL_CLOCK = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}:\d{2}:\d{2}(?:\.\d+)?)$/;

function parts(datetime: LedgerDateTime): { year: number; month: number; day: number; time: string } | null {
  const match = WALL_CLOCK.exec(datetime);
  if (!match) return null;
  return { year: Number(match[1]), month: Number(match[2]), day: Number(match[3]), time: match[4] };
}

/**
 * The day of a ledger time as a date of the browser, for a calendar to show and pick: the same year, month and day, at noon,
 * which every timezone has. The time of day is not in it.
 */
export function calendarDay(datetime: LedgerDateTime): Date | undefined {
  const it = parts(datetime);
  return it ? new Date(it.year, it.month - 1, it.day, 12) : undefined;
}

/**
 * `datetime` moved to `day`, a day picked in a calendar, keeping its time of day: picking a day must not move a transaction to
 * midnight. Without a time yet, it is `time`.
 */
export function withDay(day: Date, datetime: LedgerDateTime | undefined, time = '00:00:00'): LedgerDateTime {
  const pad = (number: number) => String(number).padStart(2, '0');
  const date = `${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`;
  return `${date}T${(datetime && parts(datetime)?.time) || time}`;
}

/** The time of day of a ledger time, `07:00:00`; `undefined` for anything else. */
export function timeOfDay(datetime: LedgerDateTime | undefined): string | undefined {
  return datetime ? parts(datetime)?.time : undefined;
}
