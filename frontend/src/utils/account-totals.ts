// The value of every node of the account tree, from the server's figures. Type-only imports keep this module runnable by
// `node --test` (account-totals.test.ts).
import BigNumber from 'bignumber.js';
import type { AccountListItem } from '@/api/types';

/** What the totals need of an account of the list: its name and the server's value of it with its sub-accounts. */
export type ValuedAccount = Pick<AccountListItem, 'name' | 'amount_with_sub_accounts'>;

/** A node's value in the operating currency (`number`, `commodity`), and its units per commodity (`detail`). */
export interface TreeTotal {
  number: BigNumber;
  commodity: string;
  detail: Record<string, BigNumber>;
}

/** The commodities a total holds a non-zero amount of. */
export function heldCommodities(total: TreeTotal | undefined): [string, BigNumber][] {
  return Object.entries(total?.detail ?? {}).filter(([, value]) => !value.isZero());
}

function fromServer(amount: ValuedAccount['amount_with_sub_accounts']): TreeTotal {
  const detail = Object.fromEntries(Object.entries(amount.detail).map(([commodity, number]) => [commodity, new BigNumber(number)]));
  return { number: new BigNumber(amount.calculated.number), commodity: amount.calculated.commodity, detail };
}

function add(total: TreeTotal | undefined, other: TreeTotal): TreeTotal {
  if (!total) return { ...other, detail: { ...other.detail } };
  const detail = { ...total.detail };
  for (const [commodity, number] of Object.entries(other.detail)) detail[commodity] = (detail[commodity] ?? new BigNumber(0)).plus(number);
  return { number: total.number.plus(other.number), commodity: total.commodity || other.commodity, detail };
}

/**
 * The value with what is under it of every node of the account tree, by path, the same whatever a page shows of the tree:
 * every account of the list counts, closed ones included.
 *
 * An account's is the server's (`amount_with_sub_accounts`, as its page shows it). A node that is no account, such as the type
 * `Assets`, or `Assets:Bank` when only `Assets:Bank:Checking` is opened, adds up the accounts right under it: those with no
 * account between them and the node, whose values hold the accounts under them already.
 */
export function treeTotals(accounts: ValuedAccount[]): Map<string, TreeTotal> {
  const names = new Set(accounts.map((account) => account.name));
  const totals = new Map<string, TreeTotal>();
  for (const account of accounts) {
    const value = fromServer(account.amount_with_sub_accounts);
    totals.set(account.name, value);
    // the nodes above the account up to the nearest account above it, which counts it already
    const parts = account.name.split(':');
    for (let depth = parts.length - 1; depth >= 1; depth--) {
      const node = parts.slice(0, depth).join(':');
      if (names.has(node)) break;
      totals.set(node, add(totals.get(node), value));
    }
  }
  return totals;
}
