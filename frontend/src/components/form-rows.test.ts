// Pure-function tests for the forms' row adapters, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
// The fixture holds the rows of `journals.payees`, `journals.accounts` and `accounts.opened` and what
// GET /api/for-new-transaction and GET /api/for-new-document answered for the same ledger
// (zhang-server/tests/fixtures/journals/probe) at the endpoint's own `now`: the adapters must give the endpoints' values.
// The row of `ledger.now` is that `now` as the query gives it, a date and a time, as both read the ledger's clock.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { accountNames, newTransactionInfo, payeeNames } from './form-rows.ts';
import { ledgerNow } from './ledger-now.ts';

interface Result {
  columns: { name: string }[];
  rows: unknown[][];
}

function rows<Row>(result: Result): Row[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column.name, row[index]])) as Row);
}

const fixture = JSON.parse(await readFile(new URL('./form-rows.fixture.json', import.meta.url), 'utf8'));

test('the new-transaction form gets what GET /api/for-new-transaction answered', () => {
  const { rows: results, expected } = fixture.newTransaction;
  const now = ledgerNow(rows(results['ledger.now']), () => assert.fail('the ledger has accounts, so ledger.now has a row'));
  assert.deepEqual(newTransactionInfo(now, rows(results['journals.payees']), rows(results['journals.accounts'])), expected);
  // not one payee or account less: the lists are the same length as the rows
  assert.equal(expected.payee.length, results['journals.payees'].rows.length);
  assert.equal(expected.account_name.length, results['journals.accounts'].rows.length);
});

test('the document upload gets what GET /api/for-new-document answered', () => {
  const { rows: results, expected } = fixture.newDocument;
  assert.deepEqual(accountNames(rows(results['accounts.opened'])), expected.account_name);
  assert.equal(expected.account_name.length, results['accounts.opened'].rows.length);
});

test('a null cell is skipped, and the order of the rows is kept', () => {
  assert.deepEqual(payeeNames([{ payee: 'b' }, { payee: null }, { payee: 'a' }]), ['b', 'a']);
  assert.deepEqual(accountNames([{ account: 'Assets:B' }, { account: null }, { account: 'Assets:A' }]), ['Assets:B', 'Assets:A']);
});
