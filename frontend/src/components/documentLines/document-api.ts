// The documents lists' and the account page's reads: the built-in queries `journals.documents`, `accounts.documents`
// and `accounts.balance_history` (POST /api/query/builtins), mapped by utils/documents.ts and utils/account-history.ts.
// A page passes the same query name and parameters to `OpenInExplore`, so what it shows and what opens there agree.
import { runBuiltin } from '@/api/requests';
import { balanceHistoryByCommodity, type BalancePoint } from '@/utils/account-history';
import { type Document, documentOf } from '@/utils/documents';

/** Every document of the ledger, newest first (`journals.documents`). */
export async function retrieveDocuments(): Promise<Document[]> {
  return (await runBuiltin('journals.documents', {})).map(documentOf);
}

/** The document directives of an account and its sub-accounts, in ledger order (`accounts.documents`). */
export async function retrieveAccountDocuments(account: string): Promise<Document[]> {
  return (await runBuiltin('accounts.documents', { account })).map(documentOf);
}

/** The balance of an account and its sub-accounts at the end of every day with a posting, per commodity (`accounts.balance_history`). */
export async function retrieveAccountBalanceHistory(account: string): Promise<Record<string, BalancePoint[]>> {
  return balanceHistoryByCommodity(await runBuiltin('accounts.balance_history', { account }));
}
