// Pure-function tests for the transaction form helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { JournalTransactionItem } from '@/api/types';
import { directiveText, parseAmount, toPostingDrafts, toPostingRequest, toRequestMetas, type TransactionFormValue } from './transaction-form-utils.ts';

type JournalPostings = JournalTransactionItem['postings'];

const unit = (number: string, commodity: string) => ({ number, commodity });

function journalPosting(account: string, number: string, metas?: { key: string; value: string }[]): JournalPostings[number] {
  return {
    account,
    unit: unit(number, 'CNY'),
    inferred_unit: unit(number, 'CNY'),
    account_before: unit('0', 'CNY'),
    account_after: unit(number, 'CNY'),
    ...(metas ? { metas } : {}),
  };
}

test('toRequestMetas trims keys, drops rows without a key and keeps values verbatim', () => {
  assert.deepEqual(
    toRequestMetas([
      { key: ' receipt ', value: ' r-1 ' },
      { key: '', value: 'orphan value' },
      { key: '   ', value: '' },
      { key: 'note', value: '' },
    ]),
    [
      { key: 'receipt', value: ' r-1 ' },
      { key: 'note', value: '' },
    ],
  );
  assert.deepEqual(toRequestMetas([]), []);
});

test('toPostingDrafts starts a new transaction with two empty postings', () => {
  assert.deepEqual(toPostingDrafts(undefined), [
    { id: 0, account: undefined, amount: '', metas: [] },
    { id: 1, account: undefined, amount: '', metas: [] },
  ]);
});

test('toPostingDrafts round-trips the metadata of each posting', () => {
  const drafts = toPostingDrafts([journalPosting('Assets:Cash', '-21', [{ key: 'receipt', value: 'r-1' }]), journalPosting('Expenses:Food', '21')]);
  assert.deepEqual(drafts, [
    { id: 0, account: 'Assets:Cash', amount: '-21 CNY', metas: [{ key: 'receipt', value: 'r-1' }] },
    { id: 1, account: 'Expenses:Food', amount: '21 CNY', metas: [] },
  ]);
});

test('toPostingDrafts keeps an elided amount empty', () => {
  const [draft] = toPostingDrafts([{ ...journalPosting('Expenses:Food', '21'), unit: null, metas: [] }]);
  assert.equal(draft.amount, '');
});

test('toPostingRequest sends the unit and the cleaned posting metadata', () => {
  const draft = {
    id: 3,
    account: 'Expenses:Food',
    amount: '21 CNY',
    metas: [
      { key: ' category ', value: 'lunch' },
      { key: '', value: '' },
    ],
  };
  assert.deepEqual(toPostingRequest(draft, parseAmount(draft.amount)), {
    account: 'Expenses:Food',
    unit: { number: '21', commodity: 'CNY' },
    metas: [{ key: 'category', value: 'lunch' }],
  });
});

test('toPostingRequest leaves an empty or invalid amount to the server and always sends metas', () => {
  const draft = { id: 0, account: undefined, amount: '', metas: [] };
  assert.deepEqual(toPostingRequest(draft, parseAmount('')), { account: '', unit: null, metas: [] });
  assert.deepEqual(toPostingRequest({ ...draft, account: 'Assets:Cash' }, parseAmount('12 USD @ 7 CNY')), { account: 'Assets:Cash', unit: null, metas: [] });
});

test('parseAmount falls back to the operating currency and rejects cost / price', () => {
  assert.deepEqual(parseAmount('-21.50', 'CNY'), { status: 'ok', number: '-21.50', commodity: 'CNY' });
  assert.deepEqual(parseAmount('-21.50'), { status: 'no_commodity' });
  assert.deepEqual(parseAmount('1 AAPL {100 USD}'), { status: 'cost_price' });
  assert.deepEqual(parseAmount('1 + 2 CNY'), { status: 'invalid' });
  assert.deepEqual(parseAmount('  '), { status: 'empty' });
});

test('directiveText writes transaction metadata first, then each posting with its own metadata', () => {
  const value: TransactionFormValue = {
    datetime: '2026-10-03T04:00:00.000Z',
    payee: 'Cafe',
    narration: 'Lunch "set"',
    flag: '*',
    postings: [
      { account: 'Assets:Cash', unit: { number: '-21', commodity: 'CNY' }, metas: [] },
      {
        account: 'Expenses:Food',
        unit: null,
        metas: [
          { key: 'zone', value: 'b' },
          { key: 'category', value: 'lunch' },
        ],
      },
    ],
    tags: ['work'],
    links: [],
    metas: [{ key: 'source', value: 'web' }],
  };
  const text = directiveText(value, {
    datetime: '2026-10-03 12:00:00',
    amounts: [parseAmount('-21 CNY'), parseAmount('')],
    invalidAmount: '<invalid>',
    accountPlaceholder: '<account>',
  });
  assert.equal(
    text,
    [
      '2026-10-03 12:00:00 * "Cafe" "Lunch \\"set\\"" #work',
      '  source: "web"',
      '  Assets:Cash -21 CNY',
      '  Expenses:Food',
      '    category: "lunch"',
      '    zone: "b"',
    ].join('\n'),
  );
});

test('directiveText shows placeholders for a missing account and an invalid amount', () => {
  const value: TransactionFormValue = {
    datetime: '',
    payee: '',
    narration: '',
    postings: [{ account: '', unit: null, metas: [] }],
    tags: [],
    links: [],
    metas: [],
  };
  const text = directiveText(value, { datetime: 'D', amounts: [parseAmount('abc')], invalidAmount: '<invalid>', accountPlaceholder: '<account>' });
  assert.equal(text, 'D * "" ""\n  <account> <invalid>');
});
