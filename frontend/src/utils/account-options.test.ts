// Pure-function tests for the account pickers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { accountOptions } from './account-options.ts';

const names = (groups: ReturnType<typeof accountOptions>) => groups.map((group) => [group.group, group.items.map((it) => it.value)]);

test('the options are the open accounts, grouped by type and sorted', () => {
  assert.deepEqual(names(accountOptions(['Expenses:Food', 'Assets:Cash', 'Assets:Bank'])), [
    ['Assets', ['Assets:Bank', 'Assets:Cash']],
    ['Expenses', ['Expenses:Food']],
  ]);
  assert.deepEqual(accountOptions(['Assets:Cash'])[0].items, [{ value: 'Assets:Cash', label: 'Assets:Cash' }]);
});

test('a closed account is not offered unless a posting already uses it', () => {
  // the server lists the open accounts only: a closed one is left out
  assert.deepEqual(names(accountOptions(['Assets:Cash'])), [['Assets', ['Assets:Cash']]]);
  // an edited transaction keeps showing the closed account it posts to, once
  assert.deepEqual(names(accountOptions(['Assets:Cash'], ['Assets:Old', undefined, 'Assets:Cash', '', 'Equity:Gone'])), [
    ['Assets', ['Assets:Cash', 'Assets:Old']],
    ['Equity', ['Equity:Gone']],
  ]);
});

test('no open account gives no options', () => {
  assert.deepEqual(accountOptions([]), []);
});
