import { fetcher } from '../global.ts';
import { JournalItem, JournalTransactionItem, Pageable } from '../rest-model';
import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { loadable_unwrap } from './index';
import { groupBy } from 'lodash-es';
import { format } from 'date-fns';

export const journalKeywordAtom = atom('');
export const journalPageAtom = atom(1);
export const journalTagsAtom = atom<string[]>([]);
export const journalLinksAtom = atom<string[]>([]);

export const journalFetcher = atomWithRefresh(async (get) => {
  const page = get(journalPageAtom);
  const keyword = get(journalKeywordAtom);
  const tags = get(journalTagsAtom);
  const links = get(journalLinksAtom);

  let url = `/api/journals?page=${page}`;

  if (keyword.trim() !== '') {
    url += `&keyword=${encodeURIComponent(keyword.trim())}`;
  }

  if (tags.length > 0) {
    url += tags.map((tag) => `&tags[]=${encodeURIComponent(tag)}`).join('');
  }

  if (links.length > 0) {
    url += links.map((link) => `&links[]=${encodeURIComponent(link)}`).join('');
  }

  const ret = await fetcher<Pageable<JournalItem>>(url);
  return ret;
});

export const journalAtom = loadable(journalFetcher);

export const groupedJournalsAtom = atom((get) => {
  return loadable_unwrap(get(journalAtom), {}, (data) => {
    return groupBy(data.records, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));
  });
});

export const previewJournalAtom = atom<JournalItem | undefined>(undefined);
export const editTransactionAtom = atom<JournalTransactionItem | undefined>(undefined);
