import { JournalItem, JournalTransactionItem } from '@/api/types';
import { format } from 'date-fns';
import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { groupBy } from 'lodash-es';
import { openAPIFetcher } from '../api/fetcher';
import { loadable_unwrap } from './index';

const findJournals = openAPIFetcher.path('/api/journals').method('get').create()


export const journalKeywordAtom = atom('');
export const journalPageAtom = atom(1);
export const journalTagsAtom = atom<string[]>([]);
export const journalLinksAtom = atom<string[]>([]);

export const journalFetcher = atomWithRefresh(async (get) => {
  const page = get(journalPageAtom);
  const keyword = get(journalKeywordAtom);
  const tags = get(journalTagsAtom);
  const links = get(journalLinksAtom);

  return (await findJournals({ page, keyword, tags, links, size: 100 })).data.data;
});



export const journalAtom = loadable(journalFetcher);

export const groupedJournalsAtom = atom((get) => {
  return loadable_unwrap(get(journalAtom), {}, (data) => {
    return groupBy(data.records, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));
  });
});

export const previewJournalAtom = atom<JournalItem | undefined>(undefined);
export const editTransactionAtom = atom<JournalTransactionItem | undefined>(undefined);
