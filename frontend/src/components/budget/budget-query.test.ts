// Pure-function tests for the budget page's "Open query" parameters, run with Node's built-in runner:
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { toBuiltinParams } from '../query/explore-link.ts';
import { budgetPostingsParams } from './budget-query.ts';

/** The parameters `budgets.postings` declares (`zhang-server/src/builtin.rs`); the server answers 400 without any of them. */
const BUDGETS_POSTINGS_PARAMS = ['accounts', 'close', 'close_time', 'month', 'name'];

const budget = { name: 'Fun', related_accounts: ['Expenses:Cinema', 'Expenses:Fun'] };

test('the activity query of a budget closed at a time sends its close date and time', () => {
  const params = toBuiltinParams(budgetPostingsParams({ ...budget, close: '2024-04-10', close_time: '12:00:00' }, new Date(2024, 3, 1)));
  assert.deepEqual(params, {
    accounts: ['Expenses:Cinema', 'Expenses:Fun'],
    month: '2024-04-01',
    name: 'Fun',
    close: '2024-04-10',
    close_time: '12:00:00',
  });
  assert.deepEqual(Object.keys(params).sort(), BUDGETS_POSTINGS_PARAMS);
});

test('the activity query of a budget closed on a day sends its close date and no time', () => {
  const params = toBuiltinParams(budgetPostingsParams({ ...budget, close: '2024-03-01', close_time: null }, new Date(2024, 0, 1)));
  assert.equal(params.close, '2024-03-01');
  assert.equal(params.close_time, null);
  assert.deepEqual(Object.keys(params).sort(), BUDGETS_POSTINGS_PARAMS);
});

test('the activity query of an open budget sends every parameter, the close as null', () => {
  for (const open of [{ ...budget, close: null, close_time: null }, budget]) {
    const params = toBuiltinParams(budgetPostingsParams(open, new Date(2025, 2, 1)));
    assert.deepEqual(params, {
      accounts: ['Expenses:Cinema', 'Expenses:Fun'],
      month: '2025-03-01',
      name: 'Fun',
      close: null,
      close_time: null,
    });
    // JSON keeps the nulls, so the server sees every parameter
    assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(params))).sort(), BUDGETS_POSTINGS_PARAMS);
  }
});
