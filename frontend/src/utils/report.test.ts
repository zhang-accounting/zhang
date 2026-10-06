// The report adapters give the figures GET /api/statistic/summary and GET /api/statistic/{type} gave, on real rows of the
// `report.*` built-ins: the fixture holds the rows and the endpoints' answers for the same ranges and types, captured on
// zhang-server/tests/fixtures/journals/probe (operating currency CNY). Run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import BigNumber from 'bignumber.js';
import {
  type AccountTotalsRow,
  type FlowsRow,
  type LiabilitiesRow,
  type NetWorthRow,
  type TopPostingsRow,
  type TransactionCountRow,
  type CalculatedAmount,
  calculatedAmount,
  reportRank,
  reportSummary,
} from './report.ts';

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
}

function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

const fixture = JSON.parse(await readFile(new URL('./report.fixture.json', import.meta.url), 'utf8'));
const CURRENCY = 'CNY';

/** The same figure by value: the endpoint summed with the engine's decimals, the adapter with BigNumber. */
function sameFigure(actual: CalculatedAmount, expected: { calculated: { number: string; commodity: string }; detail: Record<string, string> }, where: string) {
  assert.equal(actual.calculated.commodity, expected.calculated.commodity, where);
  assert.ok(new BigNumber(actual.calculated.number).eq(expected.calculated.number), `${where}: ${actual.calculated.number} vs ${expected.calculated.number}`);
  assert.deepEqual(Object.keys(actual.detail).sort(), Object.keys(expected.detail).sort(), where);
  for (const [currency, number] of Object.entries(expected.detail)) {
    assert.ok(new BigNumber(actual.detail[currency]).eq(number), `${where} ${currency}: ${actual.detail[currency]} vs ${number}`);
  }
}

test('the summary of a range is what GET /api/statistic/summary answered', () => {
  const cases = Object.entries<Record<string, Result> & { expected: Record<string, never> }>(fixture.summary);
  assert.equal(cases.length, 2);
  for (const [name, c] of cases) {
    const summary = reportSummary(
      rows<NetWorthRow>(c.net_worth),
      rows<LiabilitiesRow>(c.liabilities),
      rows<FlowsRow>(c.flows),
      rows<TransactionCountRow>(c.transaction_count),
      CURRENCY,
    );
    const expected = c.expected as unknown as Record<string, { calculated: { number: string; commodity: string }; detail: Record<string, string> }> & {
      transaction_number: number;
    };
    for (const figure of ['balance', 'liability', 'income', 'expense'] as const) {
      sameFigure(summary[figure], expected[figure], `${name} ${figure}`);
    }
    assert.equal(summary.transaction_number, expected.transaction_number, name);
  }
  // a summary with several commodities held, and a liability of nothing
  const first = fixture.summary['2024-01-01..2024-01-31'].expected;
  assert.ok(Object.keys(first.balance.detail).length >= 3);
  assert.deepEqual(first.liability.detail, {});
});

test('the rank of a type is what GET /api/statistic/{type} answered, for every type and range', () => {
  const cases = Object.entries<Record<string, Result> & { expected: { detail: unknown[]; top_transactions: Record<string, unknown>[] } }>(fixture.rank);
  assert.equal(cases.length, 10);
  let postings = 0;
  for (const [name, c] of cases) {
    const rank = reportRank(rows<AccountTotalsRow>(c.account_totals), rows<TopPostingsRow>(c.top_postings), CURRENCY);
    const expectedDetail = c.expected.detail as {
      account: string;
      amount: { calculated: { number: string; commodity: string }; detail: Record<string, string> };
    }[];
    assert.deepEqual(
      rank.detail.map((it) => it.account),
      expectedDetail.map((it) => it.account),
      name,
    );
    rank.detail.forEach((it, index) => sameFigure(it.amount, expectedDetail[index].amount, `${name} ${it.account}`));
    // the endpoint also sent the assertion fields of an account journal row, always null for a posting of the report
    const shown = c.expected.top_transactions.map(({ asserted, checked_balance, difference, tolerance, passed, ...rest }) => {
      assert.deepEqual([asserted, checked_balance, difference, tolerance, passed], [null, null, null, null, null], name);
      return rest;
    });
    assert.deepEqual(rank.top_transactions, shown, name);
    postings += shown.length;
  }
  assert.ok(postings >= 10, `${postings} postings`);
});

test('a figure merges the lots of a currency, drops what nets to zero, and sums the converted value', () => {
  const units = {
    positions: [
      { units: { number: '6', currency: 'AAPL' }, cost: { number: '10', currency: 'USD', date: '2024-01-07', label: null } },
      { units: { number: '4', currency: 'AAPL' }, cost: { number: '12', currency: 'USD', date: '2024-01-08', label: null } },
      { units: { number: '5', currency: 'EUR' }, cost: null },
      { units: { number: '-5', currency: 'EUR' }, cost: { number: '1', currency: 'USD', date: null, label: 'x' } },
      { units: { number: '744.50', currency: 'CNY' }, cost: null },
    ],
  };
  const value = {
    positions: [
      { units: { number: '1000.10', currency: 'CNY' }, cost: null },
      { units: { number: '20', currency: 'CNY' }, cost: null },
      { units: { number: '8', currency: 'HOUR' }, cost: null },
    ],
  };
  assert.deepEqual(calculatedAmount(units, value, 'CNY'), { calculated: { number: '1020.1', commodity: 'CNY' }, detail: { AAPL: '10', CNY: '744.50' } });
  // nothing held, nothing valued
  assert.deepEqual(calculatedAmount(null, null, 'CNY'), { calculated: { number: '0', commodity: 'CNY' }, detail: {} });
  assert.deepEqual(calculatedAmount({ positions: [] }, { positions: [] }, 'CNY'), { calculated: { number: '0', commodity: 'CNY' }, detail: {} });
});

test('a summary without rows is zero, and a posting the query cannot describe is left out', () => {
  const summary = reportSummary([], [], [], [], 'CNY');
  assert.deepEqual(summary, {
    balance: { calculated: { number: '0', commodity: 'CNY' }, detail: {} },
    liability: { calculated: { number: '0', commodity: 'CNY' }, detail: {} },
    income: { calculated: { number: '0', commodity: 'CNY' }, detail: {} },
    expense: { calculated: { number: '0', commodity: 'CNY' }, detail: {} },
    transaction_number: 0,
  });
  const rank = reportRank(
    [{ account: null, units: null, value: null }],
    [
      {
        date: '2024-01-05',
        time: null,
        timestamp: 1,
        account: 'Expenses:Food',
        id: 'a',
        payee: null,
        narration: null,
        units: null,
        account_balance: null,
        value: null,
      },
    ],
    'CNY',
  );
  assert.deepEqual(rank, { detail: [], top_transactions: [] });
});
