// Pure-function tests for the query chart helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { QueryResult } from '@/api/types';
import {
  buildBars,
  buildLine,
  buildSeriesBars,
  buildSeriesLines,
  buildTreemap,
  collectPoints,
  collectSeries,
  detectChartKind,
  isRunningBalance,
  localTime,
  MAX_SERIES,
  seriesCurrencies,
} from './chartData.ts';

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

// ---- multi-series charts (PIVOT BY results) ----

const eur = (number: string) => ({ number, currency: 'EUR' });
const CATEGORY = { name: 'category', type: 'str' };

function pivot(label: { name: string; type: string }, values: { name: string; type: string }[], rows: unknown[][]): QueryResult {
  return { columns: [label, ...values], rows };
}

test('pivot results are charted as grouped bars or multi-series lines', () => {
  const years = [
    { name: '2023', type: 'inventory' },
    { name: '2024', type: 'inventory' },
  ];
  const kind = (label: { name: string; type: string }, values: { name: string; type: string }[]) => detectChartKind(pivot(label, values, [['x', null, null]]));
  assert.equal(kind(CATEGORY, years), 'grouped_bar');
  assert.equal(kind(ACCOUNT, years), 'grouped_bar');
  assert.equal(kind(DATE, years), 'multi_line');
  assert.equal(
    kind(DATE, [
      { name: 'a', type: 'decimal' },
      { name: 'b', type: 'int' },
      { name: 'c', type: 'amount' },
    ]),
    'multi_line',
  );
  // the label must be a date or a string, and every other column a value
  assert.equal(kind({ name: 'year', type: 'int' }, years), null);
  assert.equal(kind(CATEGORY, [{ name: '2023', type: 'inventory' }, ACCOUNT]), null);
  assert.equal(kind(DATE, [{ name: 'flag', type: 'str' }, years[0]]), null);
  assert.equal(detectChartKind(pivot(CATEGORY, years, [])), null);
});

test('two-column results keep their chart kinds', () => {
  assert.equal(detectChartKind(result(DATE, { name: 'total', type: 'amount' }, [['2024-01-01', usd('1')]])), 'line');
  assert.equal(detectChartKind(result(ACCOUNT, { name: 'total', type: 'amount' }, [['Supermarket', usd('1')]])), 'bar');
  assert.equal(detectChartKind(result(ACCOUNT, { name: 'flag', type: 'str' }, [['a', 'b']])), null);
  assert.equal(detectChartKind({ columns: [ACCOUNT], rows: [['Assets:Bank']] }), null);
});

test('grouped bars keep the row order and leave missing cells empty', () => {
  const set = collectSeries(
    pivot(
      CATEGORY,
      [
        { name: '2023', type: 'inventory' },
        { name: '2024', type: 'inventory' },
      ],
      [
        ['Expenses:Food', inventory('10.50'), inventory('20')],
        ['Expenses:Rent', null, inventory('1000.00')],
        ['Expenses:Food', inventory('1.25'), null],
      ],
    ),
  );
  const { series, data } = buildSeriesBars(set, 'USD');
  assert.deepEqual(
    series.map((item) => item.name),
    ['2023', '2024'],
  );
  assert.deepEqual(
    data.map((datum) => [datum.label, datum.signed]),
    [
      ['Expenses:Food', ['11.75', '20']],
      ['Expenses:Rent', [null, '1000.00']],
    ],
  );
  assert.deepEqual(data[1].values, [null, 1000]);
});

test('series without a value in the currency are left out and keep their colour index', () => {
  const set = collectSeries(
    pivot(
      CATEGORY,
      [
        { name: 'shares', type: 'amount' },
        { name: 'book', type: 'amount' },
        { name: 'market', type: 'amount' },
      ],
      [
        ['Assets:Broker', { number: '10', currency: 'AAPL' }, usd('1500'), eur('1400')],
        ['Assets:Fund', null, usd('300'), usd('320')],
      ],
    ),
  );
  assert.deepEqual(seriesCurrencies(set), ['USD', 'AAPL', 'EUR']);
  const { series, data } = buildSeriesBars(set, 'USD');
  assert.deepEqual(
    series.map((item) => [item.name, item.index]),
    [
      ['book', 1],
      ['market', 2],
    ],
  );
  assert.deepEqual(
    data.map((datum) => datum.signed),
    [
      ['1500', null],
      ['300', '320'],
    ],
  );
  assert.deepEqual(
    buildSeriesBars(set, 'EUR').data.map((datum) => datum.label),
    ['Assets:Broker'],
  );
});

test('multi-series lines are sorted by date and zero-filled', () => {
  const set = collectSeries(
    pivot(
      DATE,
      [
        { name: 'Food', type: 'inventory' },
        { name: 'Rent', type: 'decimal' },
      ],
      [
        ['2024-02-01', inventory('5'), null],
        [null, inventory('99'), '99'],
        ['2024-01-01', inventory('3.50'), '1000'],
      ],
    ),
  );
  assert.deepEqual(set.labels, ['2024-02-01', '2024-01-01']);
  const food = buildSeriesLines(set, 'USD');
  assert.deepEqual(
    food.series.map((item) => item.name),
    ['Food'],
  );
  assert.deepEqual(
    food.data.map((datum) => [datum.label, datum.signed]),
    [
      ['2024-01-01', ['3.50']],
      ['2024-02-01', ['5']],
    ],
  );
  // plain numbers have no currency
  const rent = buildSeriesLines(set, '');
  assert.deepEqual(
    rent.data.map((datum) => datum.values),
    [[1000], [0]],
  );
});

test('a running balance series keeps the last row of a date', () => {
  const set = collectSeries(
    pivot(
      DATE,
      [
        { name: 'position', type: 'position' },
        { name: 'balance', type: 'inventory' },
      ],
      [
        ['2024-01-01', { units: usd('100'), cost: null }, inventory('100')],
        ['2024-01-01', { units: usd('50'), cost: null }, inventory('150')],
      ],
    ),
  );
  assert.deepEqual(buildSeriesLines(set, 'USD').data[0].signed, ['150', '150']);
  assert.deepEqual(buildSeriesLines(set, 'USD').data[0].values, [150, 150]);
});

test('at most MAX_SERIES series are drawn', () => {
  const columns = Array.from({ length: MAX_SERIES + 3 }, (_, index) => ({ name: String(2000 + index), type: 'decimal' }));
  const set = collectSeries(pivot(CATEGORY, columns, [['Food', ...columns.map(() => '1')]]));
  assert.equal(set.total, MAX_SERIES + 3);
  assert.equal(set.series.length, MAX_SERIES);
  assert.equal(buildSeriesBars(set, '').data[0].values.length, MAX_SERIES);
});
