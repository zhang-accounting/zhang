import { useEffect, useMemo, useState } from 'react';
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import { useTranslation } from 'react-i18next';
import { useDebouncedValue, useDocumentTitle } from '@mantine/hooks';
import { JournalListSkeleton } from '../components/skeletons/journalListSkeleton';
import { useAtomValue } from 'jotai/index';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { groupedJournalsAtom, journalAtom, journalFetcher, journalKeywordAtom, journalLinksAtom, journalPageAtom, journalTagsAtom } from '../states/journals';
import { useAtom, useSetAtom } from 'jotai';
import { loadable_unwrap } from '../states';
import { selectAtom } from 'jotai/utils';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { Pagination, PaginationContent, PaginationItem, PaginationLink, PaginationNext, PaginationPrevious } from '@/components/ui/pagination';
import { X } from 'lucide-react';
import { JOURNALS_LINK } from '@/layout/Sidebar';
import { TransactionPreviewModal } from '@/components/modals/TransactionPreviewModal';
import { TransactionEditModal } from '@/components/modals/TransactionEditModal';
function Journals() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { t } = useTranslation();
  const [filter, setFilter] = useState('');
  const [debouncedFilter] = useDebouncedValue(filter, 200);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Journals - ${ledgerTitle}`);

  const [journalPage, setJournalPage] = useAtom(journalPageAtom);
  const setKeyword = useSetAtom(journalKeywordAtom);
  const refreshJournals = useSetAtom(journalFetcher);
  const groupedRecords = useAtomValue(groupedJournalsAtom);
  const journalItems = useAtomValue(journalAtom);
  const total_count = useAtomValue(useMemo(() => selectAtom(journalAtom, (val) => loadable_unwrap(val, 0, (val) => val.total_count)), []));
  const total_page = useAtomValue(useMemo(() => selectAtom(journalAtom, (val) => loadable_unwrap(val, 0, (val) => val.total_page)), []));

  const [journalTags, setJournalTags] = useAtom(journalTagsAtom);
  const [journalLinks, setJournalLinks] = useAtom(journalLinksAtom);

  const removeTag = (tagToRemove: string) => {
    let newTags = journalTags.filter((tag) => tag !== tagToRemove);
    setJournalTags(newTags);
  };

  const removeLink = (linkToRemove: string) => {
    let newLinks = journalLinks.filter((tag) => tag !== linkToRemove);
    setJournalLinks(newLinks);
  };

  useEffect(() => {
    setKeyword(debouncedFilter);
  }, [setKeyword, debouncedFilter]);
  useEffect(() => {
    setBreadcrumb([JOURNALS_LINK]);
  }, []);

  const onPage = (page: number) => {
    setJournalPage(page);
  };

  return (
    <div className="flex flex-col gap-4">
      <TransactionPreviewModal />
      <TransactionEditModal />
      <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">
        {total_count} {t('JOURNALS_TITLE')}
      </h1>
      <div className="flex flex-1 items-center justify-between space-x-2">
        <div className="flex flex-1 space-x-2 items-end">
          <Input
            className="w-[33%]"
            placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
            value={filter}
            onChange={(event: any) => setFilter(event.currentTarget.value)}
          />
          {journalTags.map((tag) => (
            <Button className="pr-1" variant="secondary" size="sm" onClick={() => removeTag(tag)}>
              #{tag}
              <X className="ml-1 h-3 w-3" />
            </Button>
          ))}
          {journalLinks.map((link) => (
            <Button key={link} onClick={() => removeLink(link)} variant="secondary" size="sm" className="pr-1">
              ^{link}
              <X className="ml-1 h-3 w-3" />
            </Button>
          ))}
        </div>
        <Button variant="outline" onClick={() => refreshJournals()}>
          {t('REFRESH')}
        </Button>
      </div>

      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[100px] ">Date</TableHead>
              <TableHead className=""></TableHead>
              <TableHead className="">Payee · Narration</TableHead>
              <TableHead className="text-right ">Amount</TableHead>
              <TableHead className="text-right ">Operation</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {(journalItems.state === 'loading' || journalItems.state === 'hasError') && <JournalListSkeleton />}
            {journalItems.state === 'hasData' &&
              Object.keys(groupedRecords).map((date) => {
                return (
                  <>
                    <TableRow key={date}>
                      <TableCell colSpan={6}>
                        <span className="text-sm text-gray-500">{date}</span>
                      </TableCell>
                    </TableRow>
                    {groupedRecords[date].map((journal) => (
                      <TableViewJournalLine key={journal.id} data={journal} />
                    ))}
                  </>
                );
              })}
          </TableBody>
        </Table>
      </div>
      <Pagination className="my-4">
        <PaginationContent>
          {journalPage > 1 && (
            <PaginationItem>
              <PaginationPrevious href="#" onClick={() => onPage(journalPage - 1)} />
            </PaginationItem>
          )}
          {journalPage > 1 && (
            <PaginationItem>
              <PaginationLink href="#">{journalPage - 1}</PaginationLink>
            </PaginationItem>
          )}
          <PaginationItem>
            <PaginationLink href="#" isActive>
              {journalPage}
            </PaginationLink>
          </PaginationItem>
          {journalPage < total_page && (
            <PaginationItem>
              <PaginationLink href="#">{journalPage + 1}</PaginationLink>
            </PaginationItem>
          )}
          {journalPage < total_page && (
            <PaginationItem>
              <PaginationNext href="#" onClick={() => onPage(journalPage + 1)} />
            </PaginationItem>
          )}
        </PaginationContent>
      </Pagination>
    </div>
  );
}

export default Journals;
