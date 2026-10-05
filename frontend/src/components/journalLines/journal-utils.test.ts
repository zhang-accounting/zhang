// Pure-function tests for the journal helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { JournalBalanceCheckItem, MetaEntry } from '@/api/types';
import { isBalanceCheckPassed, journalQueryParams, journalStatus, rewriteWarning, transactionDocuments } from './journal-utils.ts';

const doc = (value: string): MetaEntry => ({ key: 'document', value });
const meta = (key: string, value: string): MetaEntry => ({ key, value });

function transaction(metas: MetaEntry[], ...postings: MetaEntry[][]) {
  return { metas, postings: postings.map((postingMetas) => ({ metas: postingMetas })) };
}

test('transactionDocuments finds a transaction-level document', () => {
  const data = transaction([meta('source', 'web'), doc('a.pdf')], [], [meta('receipt', 'r-1')]);
  assert.deepEqual(transactionDocuments(data), [doc('a.pdf')]);
});

test('transactionDocuments finds a posting-level document (an old upload after the last posting of a .bean ledger)', () => {
  const data = transaction([], [], [meta('receipt', 'r-1'), doc('receipts/b.pdf')]);
  assert.deepEqual(transactionDocuments(data), [doc('receipts/b.pdf')]);
});

test('transactionDocuments lists the transaction documents first, then those of each posting', () => {
  const data = transaction([doc('a.pdf')], [doc('b.pdf')], [doc('c.pdf'), meta('seat', 'window')]);
  assert.deepEqual(transactionDocuments(data), [doc('a.pdf'), doc('b.pdf'), doc('c.pdf')]);
});

test('transactionDocuments lists a path once when it appears several times', () => {
  const data = transaction([doc('a.pdf'), doc('a.pdf')], [doc('a.pdf'), doc('b.pdf')], [doc('b.pdf')]);
  assert.deepEqual(transactionDocuments(data), [doc('a.pdf'), doc('b.pdf')]);
});

test('transactionDocuments is empty without document metadata', () => {
  assert.deepEqual(transactionDocuments(transaction([meta('source', 'web')], [meta('receipt', 'r-1')])), []);
  assert.deepEqual(transactionDocuments(transaction([])), []);
});

/** A balance check the server checked: `checked_balance` is the balance, `asserted` the asserted amount. */
function check(balance: string, asserted: string, passed: boolean, tolerance: string | null = null): JournalBalanceCheckItem {
  const amount = (number: string) => ({ number, commodity: 'CNY' });
  const difference = String(Number(asserted) - Number(balance));
  return {
    type: 'BalanceCheck',
    type_: 'C',
    id: '00000000-0000-0000-0000-000000000000',
    sequence: 1,
    datetime: '2024-01-02T00:00:00',
    payee: 'Balance Check',
    narration: 'Assets:Bank',
    asserted: amount(asserted),
    checked_balance: amount(balance),
    difference: amount(difference),
    tolerance,
    passed,
    postings: [
      {
        account: 'Assets:Bank',
        unit: null,
        cost: null,
        inferred_unit: amount('0'),
        account_before: amount(balance),
        account_after: amount(balance),
        metas: [],
      },
    ],
  };
}

test('a balance check within its tolerance passes, as the server decided', () => {
  const data = check('50.004', '50', true, '0.01');
  assert.equal(isBalanceCheckPassed(data), true);
  assert.equal(journalStatus(data), 'ok');
});

test('a failing balance check is an error', () => {
  const data = check('165', '200', false);
  assert.equal(isBalanceCheckPassed(data), false);
  assert.equal(journalStatus(data), 'error');
});

test('journalQueryParams binds what the page filters by, and nothing for an empty filter', () => {
  assert.deepEqual(journalQueryParams(1, '', [], []), { keyword: null, tags: null, links: null, size: 100, offset: 0 });
  assert.deepEqual(journalQueryParams(3, 'Cafe', ['food'], ['trip']), {
    keyword: 'Cafe',
    tags: ['food'],
    links: ['trip'],
    size: 100,
    offset: 200,
  });
});

test('rewriteWarning asks for confirmation only when the server says an edit drops text of the transaction', () => {
  // a comment line between the postings, or on the header line, is lost when the transaction is rewritten: ask first
  assert.equal(rewriteWarning({ edit_drops_text: true }), 'edit_confirm_rewrite');
  // everything else round-trips: no dialog
  assert.equal(rewriteWarning({ edit_drops_text: false }), null);
});
