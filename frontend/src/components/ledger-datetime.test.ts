import assert from 'node:assert/strict';
import { test } from 'node:test';
import { calendarDay, timeOfDay, withDay } from './ledger-datetime.ts';

// The browser's timezone is not the ledger's: with the ledger in Asia/Shanghai and the browser in London, the form used to
// send 07:00 as the instant 07:00 London, which the server wrote as 15:00. The form keeps the wall-clock text instead.

test('an unchanged date and time is sent as the journal showed it', () => {
  const shown = '2024-01-02T07:00:00';
  const day = calendarDay(shown)!;
  assert.deepEqual([day.getFullYear(), day.getMonth(), day.getDate()], [2024, 0, 2]);
  // the calendar hands back the day it shows: the time of day is kept as written, in no timezone
  assert.equal(withDay(day, shown), shown);
});

test('picking another day keeps the time of day', () => {
  assert.equal(withDay(new Date(2024, 2, 31, 12), '2024-01-02T23:30:15'), '2024-03-31T23:30:15');
  assert.equal(withDay(new Date(2024, 2, 31, 12), '2024-01-02T07:00:00.5'), '2024-03-31T07:00:00.5');
  // a new transaction picked before the ledger's time is known
  assert.equal(withDay(new Date(2024, 2, 31, 12), undefined, '09:15:00'), '2024-03-31T09:15:00');
  assert.equal(withDay(new Date(2024, 2, 31, 12), undefined), '2024-03-31T00:00:00');
});

test('the day of a ledger time is the one written, whatever the timezone of the browser', () => {
  for (const [datetime, expected] of [
    ['2024-01-01T23:59:59', [2024, 0, 1]],
    ['2024-01-02T00:00:00', [2024, 0, 2]],
    ['2024-03-10 02:30:00', [2024, 2, 10]],
  ] as const) {
    const day = calendarDay(datetime)!;
    assert.deepEqual([day.getFullYear(), day.getMonth(), day.getDate()], expected, datetime);
  }
  assert.equal(calendarDay('2024-01-02T07:00:00Z'), undefined);
  assert.equal(timeOfDay('2024-01-02T07:00:00'), '07:00:00');
  assert.equal(timeOfDay(undefined), undefined);
});
