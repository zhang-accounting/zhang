import { loadable_unwrap } from '.';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { atom } from 'jotai';
import { openAPIFetcher } from '../api/fetcher';
import { retrieveNewDocumentInfo, retrieveNewTransactionInfo } from '../api/requests';
import { accountOptions } from '../utils/account-options';

const findPetsByStatus = openAPIFetcher.path('/api/accounts').method('get').create();

export const accountFetcher = atomWithRefresh(async () => {
  return (await findPetsByStatus({})).data.data;
});

export const accountAtom = loadable(accountFetcher);

/**
 * The accounts open now, as the server lists them (`GET /api/for-new-transaction`) by the rule the ledger checks its
 * directives with: not a closed account, nor one opened later. Reading `accountFetcher` makes them be read again
 * whenever the account list is: on a ledger reload and after a write.
 */
const openAccountsFetcher = atom(async (get) => {
  get(accountFetcher);
  return (await retrieveNewTransactionInfo({ datetime: null })).data.data.account_name;
});

const openAccountsAtom = loadable(openAccountsFetcher);

/** The options of the account pickers of what is written now, the balance tools and the document upload: the accounts open now. */
export const accountSelectItemsAtom = atom((get) => loadable_unwrap(get(openAccountsAtom), [], (names) => accountOptions(names)));

/**
 * The accounts a document written now may name, as the server lists them (`GET /api/for-new-document`): every account
 * opened by now, closed ones included, as a document only records and may follow the close. Read again whenever the
 * account list is.
 */
const documentAccountsFetcher = atom(async (get) => {
  get(accountFetcher);
  return (await retrieveNewDocumentInfo({})).data.data.account_name;
});

const documentAccountsAtom = loadable(documentAccountsFetcher);

/** The options of the document upload's account picker: every account opened by now, closed ones included. */
export const documentAccountSelectItemsAtom = atom((get) => loadable_unwrap(get(documentAccountsAtom), [], (names) => accountOptions(names)));
