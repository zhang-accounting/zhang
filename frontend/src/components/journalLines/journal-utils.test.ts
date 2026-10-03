// Pure-function tests for the journal helpers, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { MetaEntry } from '@/api/types';
import { transactionDocuments } from './journal-utils.ts';

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
