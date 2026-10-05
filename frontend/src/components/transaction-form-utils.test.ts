// Pure-function tests for the transaction form helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { JournalTransactionItem, MetaEntry } from '@/api/types';
import {
  directiveText,
  emptyDraft,
  ledgerFormat,
  metaKeyText,
  parseAmount,
  type PostingDraft,
  postingFieldErrors,
  quote,
  toPostingDrafts,
  toPostingRequest,
  toRequestMetas,
  type TransactionFormValue,
} from './transaction-form-utils.ts';

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
  assert.deepEqual(toPostingRequest({ ...row, metas: [] }, parseAmount(row.amount)).metas, documents);
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
  assert.deepEqual(toPostingRequest(row, parseAmount(row.amount)), {
    account: 'Expenses:Food',
    unit: { number: '21', commodity: 'CNY' },
    ...NO_DETAILS,
    metas: [{ key: 'category', value: 'lunch' }],
  });
});

test('toPostingRequest leaves an empty or invalid amount to the server and always sends metas', () => {
  assert.deepEqual(toPostingRequest(draft({}), parseAmount('')), { account: '', unit: null, ...NO_DETAILS, metas: [] });
  assert.deepEqual(toPostingRequest(draft({ account: 'Assets:Cash' }), parseAmount('12 USD @ 7 CNY')), {
    account: 'Assets:Cash',
    unit: null,
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
  assert.deepEqual(toPostingRequest(row, parseAmount(row.amount)), {
    account: 'Assets:Stock',
    unit: { number: '10', commodity: 'STK' },
    cost: '{7 USD}',
    price: '@@ 80 USD',
    comment: 'bought',
    metas: [],
  });
  const cleared = draft({ account: 'Assets:Stock', amount: '10 STK', cost: '', price: '  ', comment: '' });
  assert.deepEqual(toPostingRequest(cleared, parseAmount(cleared.amount)), {
    account: 'Assets:Stock',
    unit: { number: '10', commodity: 'STK' },
    ...NO_DETAILS,
    metas: [],
  });
});

test('postingFieldErrors flags a cost without braces and a price without @, and takes every ledger form', () => {
  for (const cost of ['', '{150 USD}', '{{1500 USD}}', '{}', '{ 5 USD , 2024-01-10 , "lot" }', ' {150 USD} ']) {
    assert.deepEqual(postingFieldErrors({ cost, price: '' }), {}, cost);
  }
  for (const price of ['', '@ 6 USD', '@6 USD', '@@ 60 USD']) {
    assert.deepEqual(postingFieldErrors({ cost: '', price }), {}, price);
  }
  assert.deepEqual(postingFieldErrors({ cost: '150 USD', price: '6 USD' }), { cost: 'cost_invalid', price: 'price_invalid' });
  assert.deepEqual(postingFieldErrors({ cost: '{150 USD', price: '@' }), { cost: 'cost_invalid', price: 'price_invalid' });
});

test('parseAmount falls back to the operating currency and rejects cost / price', () => {
  assert.deepEqual(parseAmount('-21.50', 'CNY'), { status: 'ok', number: '-21.50', commodity: 'CNY' });
  assert.deepEqual(parseAmount('-21.50'), { status: 'no_commodity' });
  assert.deepEqual(parseAmount('1 AAPL {100 USD}'), { status: 'cost_price' });
  assert.deepEqual(parseAmount('1 + 2 CNY'), { status: 'invalid' });
  assert.deepEqual(parseAmount('  '), { status: 'empty' });
});

test('quote escapes like the server (zhang style)', () => {
  assert.equal(quote('say "hi" \\ $5 `x`'), '"say \\"hi\\" \\\\ $5 `x`"');
  assert.equal(quote('a\tb\nc\rd'), '"a\\tb\\nc\\rd"');
  assert.equal(quote('bell\u0007 zero​width'), '"bell\\u0007 zero\\u200bwidth"');
  assert.equal(quote('旅行 café'), '"旅行 café"');
});

test('quote follows beancount escaping on beancount ledgers', () => {
  assert.equal(quote('a"b\\c\bd\fe\u0007 zero\u200bwidth', 'beancount'), '"a\\"b\\\\c\\bd\\fe\u0007 zero\u200bwidth"');
});

test('ledgerFormat reads the extension of the main file', () => {
  assert.equal(ledgerFormat(['main.bean', 'data/2024/1.zhang']), 'beancount');
  assert.equal(ledgerFormat(['main.zhang']), 'zhang');
  assert.equal(ledgerFormat(undefined), 'zhang');
  assert.equal(ledgerFormat([null]), 'zhang');
});

test('metaKeyText writes bare keys as is and quotes the others', () => {
  assert.equal(metaKeyText('receipt'), 'receipt');
  assert.equal(metaKeyText('Receipt-2.x'), 'Receipt-2.x');
  assert.equal(metaKeyText('my key'), '"my key"');
  assert.equal(metaKeyText('a:b'), '"a:b"');
  assert.equal(metaKeyText(';path'), '";path"');
  assert.equal(metaKeyText('#tag'), '"#tag"');
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
          { key: 'my key', value: 'say "hi"' },
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
      '    "my key": "say \\"hi\\""',
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

test('directiveText writes the date and a sorted time metadata on beancount ledgers', () => {
  const value: TransactionFormValue = {
    datetime: '',
    payee: 'Bob',
    narration: 'coffee',
    flag: '*',
    postings: [
      { account: 'Assets:Cash', unit: { number: '-5', commodity: 'CNY' }, metas: [{ key: 'receipt', value: 'r-1' }] },
      { account: 'Expenses:Food', unit: { number: '5', commodity: 'CNY' }, metas: [] },
    ],
    tags: [],
    links: [],
    metas: [{ key: 'zz', value: 'last' }],
  };
  const text = directiveText(value, {
    datetime: '2026-09-30 08:15:00',
    format: 'beancount',
    amounts: [parseAmount('-5 CNY'), parseAmount('5 CNY')],
    invalidAmount: '<invalid>',
    accountPlaceholder: '<account>',
  });
  assert.equal(
    text,
    ['2026-09-30 * "Bob" "coffee"', '  time: "08:15:00"', '  zz: "last"', '  Assets:Cash -5 CNY', '    receipt: "r-1"', '  Expenses:Food 5 CNY'].join('\n'),
  );
});

test('directiveText lays out a posting with its cost, price and comment like the exporter', () => {
  const value: TransactionFormValue = {
    datetime: '2024-01-10T12:00:00.000Z',
    payee: 'Broker',
    narration: 'Buy',
    flag: '*',
    postings: [
      { account: 'Assets:Stock', unit: { number: '10', commodity: 'STK' }, cost: '{ 5 USD }', price: '@ 6 USD', comment: 'inline', metas: [] },
      { account: 'Assets:Cash', unit: null, cost: null, price: null, comment: null, metas: [] },
    ],
    metas: [],
    tags: [],
    links: [],
  };
  const text = directiveText(value, {
    datetime: '2024-01-10 12:00:00',
    amounts: [parseAmount('10 STK'), parseAmount('')],
    invalidAmount: '<invalid amount>',
    accountPlaceholder: '<account>',
  });
  assert.equal(text, ['2024-01-10 12:00:00 * "Broker" "Buy"', '  Assets:Stock 10 STK { 5 USD } @ 6 USD ; inline', '  Assets:Cash'].join('\n'));
});
