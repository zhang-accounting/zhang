// Pure-function tests for the row mapping of the built-in queries, run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { rowsByColumn } from './builtin-rows.ts';

test('every cell is keyed by its column, in the order of the columns', () => {
  const columns = [
    { name: 'date', type: 'date' },
    { name: 'balance', type: 'amount' },
    { name: 'tags', type: 'set' },
  ];
  const rows = [
    ['2024-01-31', { number: '12.50', currency: 'USD' }, ['food']],
    ['2024-02-01', null, []],
  ];
  assert.deepEqual(rowsByColumn(columns, rows), [
    { date: '2024-01-31', balance: { number: '12.50', currency: 'USD' }, tags: ['food'] },
    { date: '2024-02-01', balance: null, tags: [] },
  ]);
});

test('a result without rows maps to none', () => {
  assert.deepEqual(rowsByColumn([{ name: 'payee' }], []), []);
});

test('a column named like an expression is a key as written, and a missing cell is null', () => {
  assert.deepEqual(rowsByColumn([{ name: 'sum(position)' }, { name: 'count(*)' }], [[{ positions: [] }]]), [
    { 'sum(position)': { positions: [] }, 'count(*)': null },
  ]);
});
