import { fetcher } from '..';
import { Pageable, SpanInfo } from '../rest-model';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { atom } from 'jotai';

export interface LedgerError {
  id: string;
  span: SpanInfo;
  error_type: string;
  metas: { [key: string]: string };
}

/**
 * the page to current error box
 */
export const errorPageAtom = atom(1);

export const errorsFetcher = atomWithRefresh(async (get) => {
  const page = get(errorPageAtom);
  return await fetcher<Pageable<LedgerError>>(`/api/errors?page=${page}&size=10`);
});
export const errorAtom = loadable(errorsFetcher);

export const errorCountAtom = atom((get) => {
  const errors = get(errorAtom);
  if (errors.state === 'hasError' || errors.state === 'loading') {
    return 0;
  } else {
    return errors.data.total_count;
  }
});
