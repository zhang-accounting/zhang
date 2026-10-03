import BigNumber from 'bignumber.js';

/** What the subtree totals need of an account of the list: its name and its own balance in the operating currency. */
interface ValuedAccount {
  name: string;
  amount: { calculated: { number: string } };
}

/**
 * The value of every account with its sub-accounts, by name: its own balance in the operating currency plus that of every
 * account of the list under it (`Assets:Bank:Checking` is under `Assets:Bank`, `Assets:Banking` is not), as its page shows it.
 */
export function subtreeTotals(accounts: ValuedAccount[]): Map<string, BigNumber> {
  const totals = new Map<string, BigNumber>(accounts.map((account) => [account.name, new BigNumber(0)]));
  for (const account of accounts) {
    const own = new BigNumber(account.amount.calculated.number);
    const parts = account.name.split(':');
    for (let depth = parts.length; depth >= 1; depth--) {
      const name = parts.slice(0, depth).join(':');
      const total = totals.get(name);
      if (total) totals.set(name, total.plus(own));
    }
  }
  return totals;
}
