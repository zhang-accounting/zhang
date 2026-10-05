// Pure-function tests for the balance-check rows, run with Node's built-in runner (Node >= 23.6 strips the types):
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { balanceCheckRows, batchBalanceRows, replacedBalancesText } from './balance-check.ts';

const calculated = { number: '0', commodity: 'CNY' };

/** An account as the API returns it: its own balance always has the operating currency (CNY). */
function account(name: string, own: Record<string, string>, withSubAccounts: Record<string, string>, hasSubAccounts: boolean) {
  return {
    name,
    amount: { calculated, detail: { CNY: '0', ...own } },
    balance_with_sub_accounts: withSubAccounts,
    has_sub_accounts: hasSubAccounts,
  };
}

test('a parent account is checked against the balance with its sub-accounts, labelled so', () => {
  const bank = account('Assets:Bank', { CNY: '305' }, { CNY: '500', USD: '7' }, true);
  assert.deepEqual(balanceCheckRows(bank), [
    { commodity: 'CNY', currentAmount: '500', includesSubAccounts: true },
    { commodity: 'USD', currentAmount: '7', includesSubAccounts: true },
  ]);
});

test('an account without sub-accounts is not labelled', () => {
  const cash = account('Assets:Cash', { CNY: '12' }, { CNY: '12' }, false);
  assert.deepEqual(balanceCheckRows(cash), [{ commodity: 'CNY', currentAmount: '12', includesSubAccounts: false }]);
});

test('a new account gets a row in the operating currency, at zero', () => {
  // the server gives the operating currency in both; an older one only in the own balance
  assert.deepEqual(balanceCheckRows(account('Assets:Empty', {}, {}, false)), [{ commodity: 'CNY', currentAmount: '0', includesSubAccounts: false }]);
  assert.deepEqual(balanceCheckRows(account('Assets:Empty', {}, { CNY: '0' }, false)), [{ commodity: 'CNY', currentAmount: '0', includesSubAccounts: false }]);
});

test('the batch tool lists every account, a new one too, against the balance with sub-accounts', () => {
  const rows = batchBalanceRows([
    account('Liabilities:Card', {}, {}, false),
    account('Assets:Bank', { CNY: '305' }, { CNY: '500' }, true),
    account('Assets:Bank:Checking', { CNY: '155' }, { CNY: '155' }, false),
  ]);
  assert.deepEqual(rows, [
    { accountName: 'Assets:Bank', commodity: 'CNY', currentAmount: '500', includesSubAccounts: true },
    { accountName: 'Assets:Bank:Checking', commodity: 'CNY', currentAmount: '155', includesSubAccounts: false },
    { accountName: 'Liabilities:Card', commodity: 'CNY', currentAmount: '0', includesSubAccounts: false },
  ]);
});

test('the balances a request replaced are told one per line', () => {
  const line = ({ date, account, amount }: { date: string; account: string; amount: string }) => `${account} ${date}: ${amount}`;
  assert.equal(replacedBalancesText([], line), '');
  assert.equal(
    replacedBalancesText(
      [
        { date: '2026-10-05', account: 'Assets:A', amount: { number: '100', commodity: 'CNY' }, tolerance: null },
        { date: '2026-10-05', account: 'Assets:B', amount: { number: '7', commodity: 'USD' } },
        { date: '2026-10-05', account: 'Assets:C', amount: { number: '50', commodity: 'CNY' }, tolerance: '5' },
      ],
      line,
    ),
    'Assets:A 2026-10-05: 100 CNY\nAssets:B 2026-10-05: 7 USD\nAssets:C 2026-10-05: 50 ~ 5 CNY',
  );
});
