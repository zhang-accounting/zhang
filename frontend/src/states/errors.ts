import { fetcher } from '../global.ts';
import { Pageable, SpanInfo } from '../rest-model';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { atom } from 'jotai';
import { loadable_unwrap } from './index';
import { openAPIFetcher } from '../api/requests';
export interface LedgerError {
  id: string;
  span: SpanInfo;
  error_type: string;
  metas: { [key: string]: string };
}

const findErrors = openAPIFetcher.path('/api/errors').method('get').create()

/**
 * the page to current error box
 */
export const errorPageAtom = atom(1);

export const errorsFetcher = atomWithRefresh(async (get) => {
  const page = get(errorPageAtom);
  return (await findErrors({ page, size: 10 })).data.data;
});
export const errorAtom = loadable(errorsFetcher);

export const errorCountAtom = atom((get) => {
  return loadable_unwrap(get(errorAtom), 0, (data) => data.total_count);
});
