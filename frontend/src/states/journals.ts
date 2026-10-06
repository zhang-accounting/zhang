import { JournalItem, JournalTransactionItem } from '@/api/types';
import { format } from 'date-fns';
import { atom } from 'jotai';
import { loadable } from 'jotai/utils';
import { groupBy } from 'lodash-es';
import { retrieveJournals } from '../api/requests';
import { loadable_unwrap } from './index';
import { JOURNAL_PAGE_SIZE } from '@/components/journalLines/journal-utils';
import { ledgerRevisionAtom } from './ledger';

export const journalKeywordAtom = atom('');
export const journalPageAtom = atom(1);
export const journalTagsAtom = atom<string[]>([]);
export const journalLinksAtom = atom<string[]>([]);

export const journalFetcher = atom(async (get) => {
  get(ledgerRevisionAtom);
  const page = get(journalPageAtom);
  const keyword = get(journalKeywordAtom);
  const tags = get(journalTagsAtom);
  const links = get(journalLinksAtom);

  return (await retrieveJournals({ page, keyword, tags, links, size: JOURNAL_PAGE_SIZE })).data.data;
});

export const journalAtom = loadable(journalFetcher);

export const groupedJournalsAtom = atom((get) => {
  return loadable_unwrap(get(journalAtom), {}, (data) => {
    return groupBy(data.records, (record) => format(new Date(record.datetime), 'yyyy-MM-dd'));
  });
});

/**
 * Re-reads one journal (e.g. after a document upload added a `document` meta). There is no "get by id" endpoint, so this
 * searches by the payee / narration (substring match) and pages through the results until the id turns up.
 */
export async function refetchJournal(target: JournalItem): Promise<JournalItem | undefined> {
  const keyword = target.payee || target.narration || '';
  for (let page = 1; page <= 20; page++) {
    const { data } = (await retrieveJournals({ page, keyword, tags: [], links: [], size: 100 })).data;
    const found = data.records.find((record) => record.id === target.id);
    if (found) return found;
    if (page >= data.total_page) return undefined;
  }
  return undefined;
}

/** Journal shown in `TransactionPreviewModal`; cleared when the modal unmounts or the route changes. */
export const previewJournalAtom = atom<JournalItem | undefined>(undefined);
export const editTransactionAtom = atom<JournalTransactionItem | undefined>(undefined);
