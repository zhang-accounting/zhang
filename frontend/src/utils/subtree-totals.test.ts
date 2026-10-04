import assert from 'node:assert/strict';
import { test } from 'node:test';
import { subtreeTotals } from './subtree-totals.ts';

const account = (name: string, number: string) => ({ name, amount: { calculated: { number } } });

test('an account is valued with its sub-accounts, not with a sibling that shares its prefix', () => {
  const totals = subtreeTotals([
    account('Assets:Bank', '5'),
    account('Assets:Bank:Checking', '60'),
    account('Assets:Bank:Checking:Deep', '40'),
    account('Assets:Bank:Savings', '-7.5'),
    account('Assets:Banking', '1000'),
  ]);
  assert.equal(totals.get('Assets:Bank')?.toString(), '97.5');
  assert.equal(totals.get('Assets:Bank:Checking')?.toString(), '100');
  assert.equal(totals.get('Assets:Bank:Savings')?.toString(), '-7.5');
  assert.equal(totals.get('Assets:Banking')?.toString(), '1000');
  // a parent that is no account of the list has no total
  assert.equal(totals.get('Assets'), undefined);
});
