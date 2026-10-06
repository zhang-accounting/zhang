// What the forms read of the ledger: the instant they write at and the accounts they may name then, through the built-in
// queries `ledger.now`, `journals.payees`, `journals.accounts` and `accounts.opened` (POST /api/query/builtins).
import { runBuiltin, runBuiltins } from '@/api/requests';
import type { LedgerDateTime } from './ledger-datetime';
import { type LedgerInstant, instantOf, ledgerNow } from './ledger-now';
import { type NewTransactionInfo, accountNames, newTransactionInfo } from './form-rows';

export type { NewTransactionInfo } from './form-rows';

/** The ledger's current instant (`ledger.now`), or the browser's clock for a ledger without accounts. */
export async function retrieveLedgerNow(): Promise<LedgerInstant> {
  return ledgerNow(await runBuiltin('ledger.now', {}));
}

/**
 * The payees, and the accounts open at `datetime`, the transaction's date and time as the form has it, or at the
 * ledger's current instant without one (`journals.payees` and `journals.accounts`, under one read of the ledger).
 */
export async function retrieveNewTransactionInfo(datetime?: LedgerDateTime): Promise<NewTransactionInfo> {
  const now = await retrieveLedgerNow();
  const at = datetime ? instantOf(datetime) : now;
  const [payees, accounts] = await runBuiltins([
    { name: 'journals.payees', params: {} },
    { name: 'journals.accounts', params: { date: at.date, time: at.time } },
  ]);
  return newTransactionInfo(now, payees, accounts);
}

/** The accounts open now, as the account pickers of what is written now offer them (`journals.accounts` at `ledger.now`). */
export async function retrieveOpenAccounts(): Promise<string[]> {
  const now = await retrieveLedgerNow();
  return accountNames(await runBuiltin('journals.accounts', { date: now.date, time: now.time }));
}

/**
 * The accounts a document written now may name (`accounts.opened` at `ledger.now`): every account opened by now, closed
 * ones included, as a document only records and may follow the close.
 */
export async function retrieveDocumentAccounts(): Promise<string[]> {
  const now = await retrieveLedgerNow();
  return accountNames(await runBuiltin('accounts.opened', { date: now.date, time: now.time }));
}
