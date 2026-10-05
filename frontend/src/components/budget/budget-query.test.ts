// Pure-function tests for the budget page's "Open query" parameters, run with Node's built-in runner:
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { toBuiltinParams } from '../query/explore-link.ts';
import { budgetPostingsParams } from './budget-query.ts';

/** The parameters `budgets.postings` declares (`zhang-server/src/builtin.rs`); the server answers 400 without any of them. */
const BUDGETS_POSTINGS_PARAMS = ['month', 'name'];

const budget = { name: 'Fun', related_accounts: ['Expenses:Cinema', 'Expenses:Fun'] };

test('the activity query of a budget sends its name and month, whether it is open or closed', () => {
  for (const it of [{ ...budget, close: '2024-04-10', close_time: '12:00:00' }, { ...budget, close: null, close_time: null }, budget]) {
    const params = toBuiltinParams(budgetPostingsParams(it, new Date(2024, 3, 1)));
    assert.deepEqual(params, { month: '2024-04-01', name: 'Fun' });
    assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(params))).sort(), BUDGETS_POSTINGS_PARAMS);
  }
});
