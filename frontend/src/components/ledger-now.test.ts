// Pure-function tests for the ledger's "now", run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { browserInstant, datetimeOf, instantOf, ledgerNow } from './ledger-now.ts';

test('the ledger instant is the row of ledger.now', () => {
  assert.deepEqual(ledgerNow([{ date: '2024-01-02', time: '07:30:15' }]), { date: '2024-01-02', time: '07:30:15' });
  // one row at most; a second one is never read
  assert.deepEqual(
    ledgerNow([
      { date: '2024-01-02', time: '07:30:15' },
      { date: '2030-01-01', time: '00:00:00' },
    ]),
    { date: '2024-01-02', time: '07:30:15' },
  );
});

test('without a row, or with an incomplete one, the fallback clock stands in', () => {
  const fallback = () => ({ date: '2026-10-07', time: '12:00:00' });
  assert.deepEqual(ledgerNow([], fallback), { date: '2026-10-07', time: '12:00:00' });
  assert.deepEqual(ledgerNow([{ date: null, time: '07:30:15' }], fallback), { date: '2026-10-07', time: '12:00:00' });
  assert.deepEqual(ledgerNow([{ date: '2024-01-02', time: null }], fallback), { date: '2026-10-07', time: '12:00:00' });
});

test('a wall-clock time and an instant convert both ways', () => {
  assert.deepEqual(instantOf('2024-01-02T07:30:15'), { date: '2024-01-02', time: '07:30:15' });
  assert.deepEqual(instantOf('2024-01-02T07:30:15.250'), { date: '2024-01-02', time: '07:30:15' });
  assert.deepEqual(instantOf('2024-01-02'), { date: '2024-01-02', time: '00:00:00' });
  assert.equal(datetimeOf({ date: '2024-01-02', time: '07:30:15' }), '2024-01-02T07:30:15');
});

test("the browser's clock is an instant to the second, in its own timezone", () => {
  assert.deepEqual(browserInstant(new Date(2024, 0, 2, 7, 5, 9)), { date: '2024-01-02', time: '07:05:09' });
  assert.deepEqual(browserInstant(new Date(999, 11, 31, 23, 59, 59)), { date: '0999-12-31', time: '23:59:59' });
});
