// The balance history adapter gives what GET /api/accounts/{a}/balances gave, on real rows of `accounts.balance_history`:
// the fixture holds the rows and the endpoint's answer per account, captured on zhang-server/tests/fixtures/journals/probe.
// Run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { type BalanceHistoryRow, balanceHistoryByCommodity } from './account-history.ts';

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
}

function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

const fixture = JSON.parse(await readFile(new URL('./documents.fixture.json', import.meta.url), 'utf8'));

test('the history per commodity is what GET /api/accounts/{a}/balances answered, for every account of the ledger', () => {
  const accounts = Object.entries<{ rows: Result; expected: Record<string, unknown[]> }>(fixture.balanceHistory);
  assert.ok(accounts.length >= 12);
  let points = 0;
  for (const [account, { rows: result, expected }] of accounts) {
    assert.deepEqual(balanceHistoryByCommodity(rows<BalanceHistoryRow>(result)), expected, account);
    points += Object.values(expected).reduce((n, it) => n + it.length, 0);
  }
  assert.ok(points > 20, `${points} points`);
  // an account held in two commodities has two series
  assert.deepEqual(Object.keys(fixture.balanceHistory['Expenses:Food'].expected).sort(), ['CNY', 'USD']);
});

test('a null balance is zero in the row commodity, and no rows are no series', () => {
  assert.deepEqual(balanceHistoryByCommodity([{ date: '2024-01-05', currency: 'CNY', balance: null }]), {
    CNY: [{ date: '2024-01-05', balance: { number: '0', commodity: 'CNY' } }],
  });
  assert.deepEqual(balanceHistoryByCommodity([]), {});
});
