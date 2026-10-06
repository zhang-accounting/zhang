// The budget adapters give the numbers the budget endpoints gave, on real rows of the built-in queries: the fixture holds
// the rows of `budgets.*` and the answers of `GET /api/budgets*` for the same parameters, captured on the ledger
// integration-tests/query-zhang-tables. Run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import type { Builtins } from '@/api/builtins';
import { budgetEvents, budgetInfo, budgetListItem } from './budget-rows.ts';

type MonthRow = Builtins['budgets.month']['row'];
type BudgetRow = Builtins['budgets.budget']['row'];
type EventRow = Builtins['budgets.events']['row'];
type PostingRow = Builtins['budgets.postings']['row'];

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
}

/** The rows of a captured result by column name, as `runBuiltin` gives them. */
function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

const fixture = JSON.parse(await readFile(new URL('./budget-rows.fixture.json', import.meta.url), 'utf8'));

test('the budgets of a month are those GET /api/budgets listed, with the same figures', () => {
  const months = Object.entries<{ rows: Result; expected: unknown[] }>(fixture.months);
  assert.ok(months.length >= 4);
  for (const [month, { rows: result, expected }] of months) {
    assert.deepEqual(rows<MonthRow>(result).map(budgetListItem), expected, month);
  }
});

test('a budget as of a month is what GET /api/budgets/{name} answered, before its start and after its close too', () => {
  const cases = Object.entries<{ budget: Result; month: Result; expected: unknown }>(fixture.info);
  assert.ok(cases.length >= 5);
  for (const [name, { budget, month, expected }] of cases) {
    assert.deepEqual(budgetInfo(rows<BudgetRow>(budget)[0], rows<MonthRow>(month)[0]), expected, name);
  }
  // the month before a budget's first one has no row: nothing assigned or spent, not closed
  const before = fixture.info['fun 2024-01-01'];
  assert.equal(rows(before.month).length, 1, 'fun starts in 2024-01: a month with a row');
  const start = budgetInfo(rows<BudgetRow>(before.budget)[0], undefined);
  assert.deepEqual(
    { ...start, assigned_amount: undefined, activity_amount: undefined, available_amount: undefined, closed: undefined },
    { ...before.expected, assigned_amount: undefined, activity_amount: undefined, available_amount: undefined, closed: undefined },
  );
  assert.deepEqual(
    [start?.closed, start?.assigned_amount, start?.activity_amount, start?.available_amount],
    [false, { number: '0', commodity: 'CNY' }, { number: '0', commodity: 'CNY' }, { number: '0', commodity: 'CNY' }],
  );
  // no budget, no info: the page shows "not found"
  assert.equal(budgetInfo(undefined, undefined), null);
  assert.equal(rows(fixture.info.nosuch.budget).length, 0);
});

test("a month's events and postings are those GET /api/budgets/{name}/interval listed, newest first, the budget's own entries first", () => {
  const cases = Object.entries<{ events: Result; postings: Result; expected: Record<string, unknown>[] }>(fixture.interval);
  assert.ok(cases.length >= 4);
  for (const [name, { events, postings, expected }] of cases) {
    // the page reads these fields; the endpoint also sent the assertion fields, always null for a budget's postings
    const shown = expected.map(({ asserted, checked_balance, difference, tolerance, passed, ...rest }) => {
      assert.deepEqual(
        [asserted, checked_balance, difference, tolerance, passed],
        rest.type === 'Posting' ? [null, null, null, null, null] : [undefined, undefined, undefined, undefined, undefined],
      );
      return rest;
    });
    assert.deepEqual(budgetEvents(rows<EventRow>(events), rows<PostingRow>(postings)), shown, name);
  }
});

test('an event at the same time as a posting comes first, and a missing narration is null', () => {
  const events = budgetEvents(
    [{ date: '2024-03-02', time: '10:00:00', timestamp: 200, type: 'assign', amount: { number: '5', currency: 'CNY' } }],
    [
      {
        date: '2024-03-03',
        time: '00:00:00',
        timestamp: 300,
        account: 'Expenses:Food',
        id: 'a',
        payee: 'Shop',
        narration: '',
        units: { number: '1', currency: 'CNY' },
        balance: null,
      },
      {
        date: '2024-03-02',
        time: '10:00:00',
        timestamp: 200,
        account: 'Expenses:Food',
        id: 'b',
        payee: null,
        narration: 'x',
        units: null,
        balance: { number: '2', currency: 'CNY' },
      },
    ],
  );
  assert.deepEqual(
    events.map((it) => [it.type, it.timestamp, 'narration' in it ? it.narration : it.event_type, 'account_after' in it ? it.account_after : it.amount]),
    [
      ['Posting', 300, null, { number: '0', commodity: 'CNY' }],
      ['BudgetEvent', 200, 'AddAssignedAmount', { number: '5', commodity: 'CNY' }],
      ['Posting', 200, 'x', { number: '2', commodity: 'CNY' }],
    ],
  );
});
