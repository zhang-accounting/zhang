// Pure-function tests for the transaction form helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { JournalTransactionItem, MetaEntry } from '@/api/types';
import { emptyDraft, type PostingDraft, toPostingDrafts, toPostingRequest, toRequestMetas } from './transaction-form-utils.ts';

type JournalPostings = JournalTransactionItem['postings'];

const unit = (number: string, commodity: string) => ({ number, commodity });

type Written = NonNullable<JournalPostings[number]['written']>;

function journalPosting(account: string, number: string, metas: MetaEntry[] = [], written: Written = {}): JournalPostings[number] {
  return {
    account,
    unit: unit(number, 'CNY'),
    inferred_unit: unit(number, 'CNY'),
    account_before: unit('0', 'CNY'),
    account_after: unit(number, 'CNY'),
    metas,
    written,
  };
}

function draft(fields: Partial<PostingDraft>): PostingDraft {
  return { ...emptyDraft(0), ...fields };
}

/** The cost, price and comment fields of a request posting, which the form always sends (`null` for an empty one). */
const NO_DETAILS = { cost: null, price: null, comment: null };

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
  assert.deepEqual(toPostingDrafts(undefined), [draft({ id: 0 }), draft({ id: 1 })]);
});

test('toPostingDrafts round-trips the metadata of each posting', () => {
  const drafts = toPostingDrafts([journalPosting('Assets:Cash', '-21', [{ key: 'receipt', value: 'r-1' }]), journalPosting('Expenses:Food', '21')]);
  assert.deepEqual(drafts, [
    draft({ id: 0, account: 'Assets:Cash', amount: '-21 CNY', metas: [{ key: 'receipt', value: 'r-1' }] }),
    draft({ id: 1, account: 'Expenses:Food', amount: '21 CNY' }),
  ]);
});

test('toPostingDrafts keeps a posting document out of the editor and sends it back unchanged', () => {
  const documents = [{ key: 'document', value: 'receipts/a.pdf' }];
  const [row] = toPostingDrafts([journalPosting('Expenses:Food', '21', [...documents, { key: 'receipt', value: 'r-1' }])]);
  assert.deepEqual(row.metas, [{ key: 'receipt', value: 'r-1' }]);
  assert.deepEqual(row.documents, documents);
  assert.deepEqual(toPostingRequest({ ...row, metas: [] }).metas, documents);
});

test('toPostingDrafts keeps an elided amount empty', () => {
  const [row] = toPostingDrafts([{ ...journalPosting('Expenses:Food', '21'), unit: null }]);
  assert.equal(row.amount, '');
});

test('toPostingRequest sends the unit and the cleaned posting metadata', () => {
  const row = draft({
    id: 3,
    account: 'Expenses:Food',
    amount: '21 CNY',
    metas: [
      { key: ' category ', value: 'lunch' },
      { key: '', value: '' },
    ],
  });
  assert.deepEqual(toPostingRequest(row), {
    account: 'Expenses:Food',
    unit: '21 CNY',
    ...NO_DETAILS,
    metas: [{ key: 'category', value: 'lunch' }],
  });
});

test('toPostingRequest leaves an empty or invalid amount to the server and always sends metas', () => {
  assert.deepEqual(toPostingRequest(draft({})), { account: '', unit: null, ...NO_DETAILS, metas: [] });
  assert.deepEqual(toPostingRequest(draft({ account: 'Assets:Cash', amount: ' 12 USD @ 7 CNY ' })), {
    account: 'Assets:Cash',
    unit: '12 USD @ 7 CNY',
    ...NO_DETAILS,
    metas: [],
  });
});

// A posting with a cost or a price is editable (#443): the form shows them as the journal has them written, and sends them.

test('toPostingDrafts shows the cost, price and comment of a posting as written, and none as empty fields', () => {
  const [stock, cash] = toPostingDrafts([
    { ...journalPosting('Assets:Stock', '10', [], { cost: '{ 5 USD }', price: '@ 6 USD', comment: 'inline' }), unit: unit('10', 'STK') },
    { ...journalPosting('Assets:Cash', '-50'), unit: null },
  ]);
  assert.deepEqual(stock, draft({ id: 0, account: 'Assets:Stock', amount: '10 STK', cost: '{ 5 USD }', price: '@ 6 USD', comment: 'inline' }));
  assert.deepEqual(cash, draft({ id: 1, account: 'Assets:Cash', amount: '' }));
  // an entry without a written form (a balance check's, or an old server) has none either
  const [plain] = toPostingDrafts([{ ...journalPosting('Assets:Cash', '-50'), written: null }]);
  assert.deepEqual(plain, draft({ id: 0, account: 'Assets:Cash', amount: '-50 CNY' }));
});

test('toPostingRequest sends the cost, price and comment as typed, trimmed, and null for an emptied one', () => {
  const row = draft({ account: 'Assets:Stock', amount: '10 STK', cost: ' {7 USD}', price: '@@ 80 USD ', comment: 'bought' });
  assert.deepEqual(toPostingRequest(row), {
    account: 'Assets:Stock',
    unit: '10 STK',
    cost: '{7 USD}',
    price: '@@ 80 USD',
    comment: 'bought',
    metas: [],
  });
  const cleared = draft({ account: 'Assets:Stock', amount: '10 STK', cost: '', price: '  ', comment: '' });
  assert.deepEqual(toPostingRequest(cleared), {
    account: 'Assets:Stock',
    unit: '10 STK',
    ...NO_DETAILS,
    metas: [],
  });
});
