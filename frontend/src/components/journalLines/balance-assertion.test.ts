// Tests of the one shape of a balance assertion in both journals, run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { JournalBalanceCheckItem } from '@/api/types';
import { assertionOf } from './balance-assertion.ts';

const amount = (number: string) => ({ number, commodity: 'CNY' });

/** `balance Assets:Bank 100 CNY` checked against a balance of 90 CNY, as `GET /api/journals` lists it. */
const JOURNAL_ITEM: JournalBalanceCheckItem = {
  type: 'BalanceCheck',
  type_: 'C',
  id: '00000000-0000-0000-0000-000000000000',
  sequence: 1,
  datetime: '2024-01-02T00:00:00',
  payee: 'Balance Check',
  narration: 'Assets:Bank',
  postings: [
    {
      account: 'Assets:Bank',
      unit: null,
      cost: null,
      inferred_unit: amount('0'),
      account_before: amount('90'),
      account_after: amount('90'),
      metas: [],
    },
  ],
  asserted: amount('100'),
  checked_balance: amount('90'),
  difference: amount('10'),
  tolerance: null,
  passed: false,
};

/** The same assertion as `GET /api/accounts/Assets:Bank/journals` lists it. */
const ACCOUNT_ROW = {
  datetime: '2024-01-02T00:00:00',
  timestamp: 1704153600,
  account: 'Assets:Bank',
  trx_id: '00000000-0000-0000-0000-000000000000',
  payee: 'Balance Check',
  narration: 'Assets:Bank',
  inferred_unit: amount('0'),
  account_after: amount('90'),
  asserted: amount('100'),
  checked_balance: amount('90'),
  difference: amount('10'),
  tolerance: null,
  passed: false,
};

test('an assertion reads the same from the journal and from an account journal', () => {
  const expected = { asserted: amount('100'), checked_balance: amount('90'), difference: amount('10'), tolerance: null, passed: false };
  assert.deepEqual(assertionOf(JOURNAL_ITEM), expected);
  assert.deepEqual(assertionOf(ACCOUNT_ROW), expected);
});

test('an assertion within its tolerance keeps its tolerance and passes', () => {
  const within = { ...ACCOUNT_ROW, asserted: amount('90.004'), difference: amount('0.004'), tolerance: '0.01', passed: true };
  assert.deepEqual(assertionOf(within), {
    asserted: amount('90.004'),
    checked_balance: amount('90'),
    difference: amount('0.004'),
    tolerance: '0.01',
    passed: true,
  });
});

test('a posting of an account journal is no assertion', () => {
  const posting = {
    ...ACCOUNT_ROW,
    payee: 'Shop',
    inferred_unit: amount('-5'),
    account_after: amount('85'),
    asserted: null,
    checked_balance: null,
    difference: null,
    passed: null,
  };
  assert.equal(assertionOf(posting), null);
});
