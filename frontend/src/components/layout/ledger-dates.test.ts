// Pure-function tests for the report's ledger dates, run with Node's built-in runner:
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { endOfDay, endOfMonth, startOfDay, startOfMonth } from 'date-fns';
import { ledgerDates } from './ledger-dates.ts';

test('a range is sent as the days picked, both inclusive, from the first to the last instant of them', () => {
  const april = new Date(2025, 3, 15, 12, 30);
  assert.deepEqual(ledgerDates({ from: startOfMonth(april), to: endOfMonth(april) }), { from: '2025-04-01', to: '2025-04-30' });
  // the old UI sent the first second of the 1st, which dropped everything on that day (#400)
  assert.deepEqual(ledgerDates({ from: new Date(2025, 3, 1, 0, 0, 1), to: endOfDay(april) }), { from: '2025-04-01', to: '2025-04-15' });
});

test('a single day is a range of one day', () => {
  const day = new Date(2024, 1, 29, 23, 59, 59, 999);
  assert.deepEqual(ledgerDates({ from: startOfDay(day), to: endOfDay(day) }), { from: '2024-02-29', to: '2024-02-29' });
});
