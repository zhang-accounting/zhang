import { loadable } from 'jotai/utils';
import { atom } from 'jotai';
import { loadable_unwrap } from './index';
import { runBuiltinWithTotal } from '../api/requests';
import { ledgerErrorPage } from '../utils/ledger-errors';
import { ledgerRevisionAtom } from './ledger';

/** the errors on one page of the error box */
const PAGE_SIZE = 10;

/**
 * the page to current error box
 */
export const errorPageAtom = atom(1);

/** The current page of the ledger's errors: the built-in query `journals.errors`, counted (`total`) for the pager. */
export const errorsFetcher = atom(async (get) => {
  get(ledgerRevisionAtom);
  const page = get(errorPageAtom);
  const { rows, total } = await runBuiltinWithTotal('journals.errors', { size: PAGE_SIZE, offset: (page - 1) * PAGE_SIZE });
  return ledgerErrorPage(rows, total, page, PAGE_SIZE);
});
export const errorAtom = loadable(errorsFetcher);

export const errorCountAtom = atom((get) => {
  return loadable_unwrap(get(errorAtom), 0, (data) => data.total_count);
});
