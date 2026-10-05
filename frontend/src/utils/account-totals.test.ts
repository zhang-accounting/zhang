import assert from 'node:assert/strict';
import { test } from 'node:test';
import { heldCommodities, treeTotals, type ValuedAccount } from './account-totals.ts';

/** An account of the list with the server's value of it with its sub-accounts. */
function account(name: string, number: string, detail: Record<string, string> = { CNY: number }): ValuedAccount {
  return { name, amount_with_sub_accounts: { calculated: { number, commodity: 'CNY' }, detail } };
}

const text = (totals: ReturnType<typeof treeTotals>, path: string) => totals.get(path)?.number.toString();

test("an account's total is the server's value of it with its sub-accounts", () => {
  // the server's Assets:Bank holds its sub-accounts already, and a closed one that still holds money
  const totals = treeTotals([account('Assets:Bank', '115'), account('Assets:Bank:Checking', '100'), account('Assets:Bank:Old', '10')]);
  assert.equal(text(totals, 'Assets:Bank'), '115');
  assert.equal(text(totals, 'Assets:Bank:Checking'), '100');
  // the type adds up the accounts right under it, not their sub-accounts again
  assert.equal(text(totals, 'Assets'), '115');
});

test('a node that is no account adds up the accounts right under it, not a sibling that shares its prefix', () => {
  const totals = treeTotals([
    account('Assets:Bank:Checking', '100', { CNY: '86', USD: '2' }),
    account('Assets:Bank:Checking:Deep', '40'),
    account('Assets:Bank:Savings', '-7.5'),
    account('Assets:Banking', '1000'),
    account('Liabilities:Card', '-20'),
  ]);
  // Assets:Bank:Checking holds Deep already
  assert.equal(text(totals, 'Assets:Bank'), '92.5');
  assert.equal(text(totals, 'Assets'), '1092.5');
  assert.equal(text(totals, 'Liabilities'), '-20');
  assert.deepEqual(
    heldCommodities(totals.get('Assets:Bank')).map(([commodity, number]) => [commodity, number.toString()]),
    [
      ['CNY', '78.5'],
      ['USD', '2'],
    ],
  );
});
