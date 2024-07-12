import { fetcher } from '..';
import { JournalItem, Pageable } from '../rest-model';
import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { loadable_unwrap } from './index';
import { groupBy } from 'lodash-es';
import { format } from 'date-fns';

export const journalKeywordAtom = atom('');
export const journalPageAtom = atom(1);

export const journalFetcher = atomWithRefresh(async (get) => {
  const page = get(journalPageAtom);
  const keyword = get(journalKeywordAtom);
  const url = keyword.trim() === '' ? `/api/journals?page=${page}` : `/api/journals?page=${page}&keyword=${keyword.trim()}`;
  const ret = await fetcher<Pageable<JournalItem>>(url);
  return ret;
});

export const journalAtom = loadable(journalFetcher);

export const groupedJournalsAtom = atom((get) => {
  return loadable_unwrap(get(journalAtom), {}, (data) => {
    return groupBy(data.records, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));
  });
});
