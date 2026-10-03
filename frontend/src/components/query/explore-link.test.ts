// Pure-function tests for the "Open in Explore" helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { exploreUrl, ledgerDate, queryFromSearch, toBuiltinParams } from './explore-link.ts';

test('a query survives the round trip through the explore URL', () => {
  for (const query of [
    'SELECT date, payee WHERE payee = "O\'Brien" AND date >= 2024-01-01',
    "SELECT * WHERE payee = ('it' + '\"s') -- 100% & more\nORDER BY seq",
    "SELECT 1 + 2, 'C:\\temp\\' AS path, set('a#b', 'c?d')",
    '查询 午餐 😀',
  ]) {
    const url = exploreUrl(query);
    assert.ok(url.startsWith('/explore?query='), url);
    // nothing that would end the parameter or start a fragment is left unencoded
    assert.ok(!/[\s#&]/.test(url.slice('/explore?query='.length)), url);
    assert.equal(queryFromSearch(url.slice('/explore'.length)), query);
    assert.equal(queryFromSearch(new URL(url, 'http://localhost').searchParams), query);
  }
});

test('an explore URL without a query has none', () => {
  assert.equal(queryFromSearch(''), null);
  assert.equal(queryFromSearch('?tab=chart'), null);
  assert.equal(queryFromSearch('?query='), null);
  assert.equal(queryFromSearch('?query=%20%0A'), null);
  assert.equal(queryFromSearch('?query=SELECT+1'), 'SELECT 1');
});

test('dates are sent as the day the page shows', () => {
  assert.equal(ledgerDate(new Date(2024, 0, 1)), '2024-01-01');
  // the last moment of a day is still that day, whatever the browser timezone
  assert.equal(ledgerDate(new Date(2024, 1, 29, 23, 59, 59, 999)), '2024-02-29');
  assert.equal(ledgerDate(new Date(2024, 11, 31, 0, 0, 1)), '2024-12-31');
  assert.equal(ledgerDate(new Date(987, 5, 7)), '0987-06-07');
});

test('built-in query parameters are sent as JSON values of their types', () => {
  assert.deepEqual(
    toBuiltinParams({
      from: new Date(2024, 0, 1),
      to: new Date(2024, 0, 31, 23, 59, 59),
      tags: new Set(['trip', 'food']),
      accounts: ['Assets:Bank'],
      payee: "O'Brien",
      size: 50,
      amount: '12.50',
      cleared: true,
      keyword: null,
    }),
    {
      from: '2024-01-01',
      to: '2024-01-31',
      tags: ['trip', 'food'],
      accounts: ['Assets:Bank'],
      payee: "O'Brien",
      size: 50,
      amount: '12.50',
      cleared: true,
      keyword: null,
    },
  );
  assert.deepEqual(toBuiltinParams({}), {});
});
