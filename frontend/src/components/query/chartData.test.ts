// Pure-function tests for the query chart helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { QueryResult } from '@/api/types';
import { buildBars, buildLine, buildTreemap, collectPoints, detectChartKind, isRunningBalance, localTime } from './chartData.ts';

const usd = (number: string) => ({ number, currency: 'USD' });
const inventory = (...numbers: string[]) => ({ positions: numbers.map((number) => ({ units: usd(number), cost: null })) });

function result(label: { name: string; type: string }, value: { name: string; type: string }, rows: unknown[][]): QueryResult {
  return { columns: [label, value], rows };
}

const DATE = { name: 'date', type: 'date' };
const ACCOUNT = { name: 'account', type: 'str' };

test('rows sharing a label are summed for flows', () => {
  const points = collectPoints(
    result(DATE, { name: 'position', type: 'position' }, [
      ['2024-01-01', { units: usd('100.00'), cost: null }],
      ['2024-01-01', { units: usd('150.5'), cost: null }],
      ['2024-01-02', { units: usd('-20'), cost: null }],
    ]),
  );
  assert.deepEqual(
    buildLine(points, 'USD').map((datum) => [datum.date, datum.signed]),
    [
      ['2024-01-01', '250.50'],
      ['2024-01-02', '-20'],
    ],
  );
});

test('a running balance keeps the last row of a label', () => {
  for (const name of ['balance', 'cost(balance)', 'units(balance)']) {
    const points = collectPoints(
      result(DATE, { name, type: 'inventory' }, [
        ['2024-01-01', inventory('100')],
        ['2024-01-01', inventory('150')],
        ['2024-01-02', inventory('130')],
      ]),
    );
    assert.deepEqual(
      buildLine(points, 'USD').map((datum) => datum.value),
      [150, 130],
      name,
    );
  }
});

test('running balance columns are recognised by name', () => {
  assert.equal(isRunningBalance('balance'), true);
  assert.equal(isRunningBalance('cost(balance)'), true);
  assert.equal(isRunningBalance('position'), false);
  assert.equal(isRunningBalance('sum(position)'), false);
  assert.equal(isRunningBalance('balances'), false);
});

test('NULL values are skipped and do not reset a running balance', () => {
  const points = collectPoints(
    result(DATE, { name: 'balance', type: 'inventory' }, [
      ['2024-01-01', inventory('100')],
      ['2024-01-01', null],
    ]),
  );
  assert.deepEqual(
    buildLine(points, 'USD').map((datum) => datum.signed),
    ['100'],
  );
});

test('exact values keep the scale of their source strings', () => {
  const points = collectPoints(
    result(ACCOUNT, { name: 'sum(position)', type: 'inventory' }, [
      ['Assets:Bank', inventory('2754.10')],
      ['Assets:Cash', inventory('1.10', '2.20')],
    ]),
  );
  assert.deepEqual(
    buildBars(points, 'USD').map((bar) => bar.signed),
    ['2754.10', '3.30'],
  );
  const leaves = buildTreemap(points, 'USD').nodes.map((node) => node.signed);
  assert.deepEqual(leaves, ['2754.10', '3.30']);
});

test('treemaps are chosen only for account-like labels', () => {
  const kind = (...labels: string[]) =>
    detectChartKind(
      result(
        ACCOUNT,
        { name: 'total', type: 'amount' },
        labels.map((label) => [label, usd('1')]),
      ),
    );
  assert.equal(kind('Assets:Bank:Checking', 'Expenses:Food'), 'treemap');
  assert.equal(kind('Assets', 'Assets:Bank'), 'treemap');
  assert.equal(kind('Assets:银行', 'Assets:Bank:2024'), 'treemap');
  assert.equal(kind('10:30', '11:00'), 'bar');
  assert.equal(kind('https://example.com/a', 'Assets:Bank'), 'bar');
  assert.equal(kind('Assets:Bank Account'), 'bar');
  assert.equal(kind('Assets', 'Expenses'), 'bar');
  assert.equal(kind('Supermarket', 'Cafe'), 'bar');
});

test('dates before year 100 are not mapped to the 1900s', () => {
  const time = localTime('0050-03-04');
  assert.notEqual(time, null);
  const date = new Date(time as number);
  assert.deepEqual([date.getFullYear(), date.getMonth(), date.getDate(), date.getHours()], [50, 2, 4, 0]);
  assert.equal(localTime('2024-1-1'), null);
});

test('line points are sorted by date', () => {
  const points = collectPoints(
    result(DATE, { name: 'amount', type: 'amount' }, [
      ['2024-02-01', usd('2')],
      ['2024-01-01', usd('1')],
    ]),
  );
  assert.deepEqual(
    buildLine(points, 'USD').map((datum) => datum.date),
    ['2024-01-01', '2024-02-01'],
  );
});
