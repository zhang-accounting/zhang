// Type-only imports keep this module runnable by `node --test` (balance-check.test.ts).
import type { Account, AccountListItem } from '@/api/types';

/** What the balance-check forms need of an account: the account page's info or a row of the account list. */
type AccountBalances = Pick<Account | AccountListItem, 'name' | 'amount' | 'balance_with_sub_accounts' | 'has_sub_accounts'>;

/** One commodity a `balance` on the account can assert, with the balance it would be checked against. */
export interface BalanceCheckRow {
  commodity: string;
  /** The balance a `balance` on the account is checked against: that of the account and all its sub-accounts. */
  currentAmount: string;
  /** The account has sub-accounts, which `currentAmount` includes: label it so. */
  includesSubAccounts: boolean;
}

/**
 * The commodities a balance check of the account offers, sorted: those the account or its sub-accounts hold, and those of its
 * own balance, which always has the operating currency, so a new account can set its opening balance. Each is valued with
 * the balance a `balance` checks, with the sub-accounts, or 0 when they hold none of it.
 */
export function balanceCheckRows(account: AccountBalances): BalanceCheckRow[] {
  const commodities = new Set([...Object.keys(account.balance_with_sub_accounts), ...Object.keys(account.amount.detail)]);
  return [...commodities].sort().map((commodity) => ({
    commodity,
    currentAmount: account.balance_with_sub_accounts[commodity] ?? '0',
    includesSubAccounts: account.has_sub_accounts,
  }));
}

/** The rows of the batch balance tool: every account's balance-check rows, sorted by account, then commodity. */
export function batchBalanceRows(accounts: AccountBalances[]): (BalanceCheckRow & { accountName: string })[] {
  return accounts
    .flatMap((account) => balanceCheckRows(account).map((row) => ({ ...row, accountName: account.name })))
    .sort((a, b) => a.accountName.localeCompare(b.accountName) || a.commodity.localeCompare(b.commodity));
}

/** A balance a request replaced: one of the same account and commodity for the same date (`replaced` of the answer). */
export interface ReplacedBalance {
  date: string;
  account: string;
  amount: { number: string; commodity: string };
  /** the tolerance (`~`) it was written with, which the new, exact balance does not keep */
  tolerance?: string | null;
}

/** How a toast says which balances a request replaced, one per line, each with the amount it asserted, written as in the
 * ledger (`100 ~ 5 CNY` with a tolerance); empty when it replaced none. */
export function replacedBalancesText(replaced: ReplacedBalance[], line: (balance: { date: string; account: string; amount: string }) => string): string {
  return replaced
    .map((it) => {
      const tolerance = it.tolerance ? ` ~ ${it.tolerance}` : '';
      return line({ date: it.date, account: it.account, amount: `${it.amount.number}${tolerance} ${it.amount.commodity}` });
    })
    .join('\n');
}
