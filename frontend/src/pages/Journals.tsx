import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { NotebookText, RefreshCw, Search, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { parseISO } from 'date-fns';
import { journalQueryParams } from '@/components/journalLines/journal-utils';
import { JournalRow } from '@/components/journalLines/JournalRow';
import { EmptyState, LoadFailedState, PageHeader, PageShell } from '@/components/layout';
import { PagePagination } from '@/components/layout/PagePagination';
import { useDateFormat } from '@/components/layout/use-date-format';
import { TransactionEditModal } from '@/components/modals/TransactionEditModal';
import { TransactionPreviewModal } from '@/components/modals/TransactionPreviewModal';
import { OpenInExplore } from '@/components/query/OpenInExplore';
import { JournalDaysSkeleton } from '@/components/skeletons/journalListSkeleton';
import { Button } from '@/components/ui/button';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { useDebouncedValue } from '@/hooks/use-debounced';
import { cn } from '@/lib/utils';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { JOURNALS_LINK } from '@/layout/nav-links';
import { breadcrumbAtom, titleAtom } from '../states/basic';
import { groupedJournalsAtom, journalAtom, journalFetcher, journalKeywordAtom, journalLinksAtom, journalPageAtom, journalTagsAtom } from '../states/journals';

function Journals() {
  const { t } = useTranslation();
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${t('NAV_JOURNALS')} - ${ledgerTitle}`);

  const [keyword, setKeyword] = useAtom(journalKeywordAtom);
  const [filter, setFilter] = useState(keyword);
  const [debouncedFilter] = useDebouncedValue(filter, 200);
  const [journalPage, setJournalPage] = useAtom(journalPageAtom);
  const [journalTags, setJournalTags] = useAtom(journalTagsAtom);
  const [journalLinks, setJournalLinks] = useAtom(journalLinksAtom);
  const refreshJournals = useSetAtom(journalFetcher);
  const journalItems = useAtomValue(journalAtom);

  const data = journalItems.state === 'hasData' ? journalItems.data : undefined;
  const hasFilters = filter.trim() !== '' || journalTags.length > 0 || journalLinks.length > 0;

  useEffect(() => {
    setBreadcrumb([JOURNALS_LINK]);
  }, [setBreadcrumb]);

  useEffect(() => {
    if (debouncedFilter === keyword) return;
    setKeyword(debouncedFilter);
    setJournalPage(1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedFilter]);

  const onPage = (page: number) => {
    setJournalPage(page);
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  const clearFilters = () => {
    setFilter('');
    setKeyword('');
    setJournalTags([]);
    setJournalLinks([]);
    setJournalPage(1);
  };

  let content;
  if (journalItems.state === 'hasError') {
    content = <LoadFailedState description={String(journalItems.error)} onRetry={refreshJournals} />;
  } else if (journalItems.state === 'loading') {
    content = <JournalDaysSkeleton />;
  } else if ((data?.records.length ?? 0) === 0) {
    content = (
      <EmptyState
        icon={NotebookText}
        title={hasFilters ? t('ledger.journals.no_match_title') : t('ledger.journals.empty_title')}
        description={hasFilters ? t('ledger.journals.no_match_description') : t('ledger.journals.empty_description')}
        action={
          hasFilters && (
            <Button variant="outline" className="h-10 md:h-8" onClick={clearFilters}>
              {t('ledger.common.clear_filters')}
            </Button>
          )
        }
      />
    );
  } else {
    content = <JournalDays />;
  }

  return (
    <PageShell>
      <TransactionPreviewModal />
      <TransactionEditModal />
      <PageHeader
        title={t('NAV_JOURNALS')}
        description={data ? t('ledger.journals.description', { count: data.total_count }) : t('ledger.journals.description_loading')}
        actions={<OpenInExplore name="journals.page" params={journalQueryParams(journalPage, keyword, journalTags, journalLinks)} />}
      />

      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <InputGroup className="h-10 flex-1 md:h-8 md:max-w-sm">
            <InputGroupAddon>
              <Search />
            </InputGroupAddon>
            <InputGroupInput
              type="text"
              enterKeyHint="search"
              placeholder={t('ledger.journals.search_placeholder')}
              aria-label={t('ledger.journals.search_placeholder')}
              value={filter}
              onChange={(event) => setFilter(event.currentTarget.value)}
            />
            {filter && (
              <InputGroupAddon align="inline-end">
                <InputGroupButton size="icon-xs" aria-label={t('ledger.common.clear')} onClick={() => setFilter('')}>
                  <X />
                </InputGroupButton>
              </InputGroupAddon>
            )}
          </InputGroup>
          <Button variant="outline" className="h-10 md:ml-auto md:h-8" aria-label={t('REFRESH')} onClick={() => refreshJournals()}>
            <RefreshCw data-icon="inline-start" className={journalItems.state === 'loading' ? 'animate-spin' : undefined} />
            <span className="hidden sm:inline">{t('REFRESH')}</span>
          </Button>
        </div>
        {(journalTags.length > 0 || journalLinks.length > 0) && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs text-muted-foreground">{t('ledger.journals.filtered_by')}</span>
            {journalTags.map((tag) => (
              <FilterChip
                key={`tag-${tag}`}
                label={`#${tag}`}
                onRemove={() => {
                  setJournalTags(journalTags.filter((it) => it !== tag));
                  setJournalPage(1);
                }}
              />
            ))}
            {journalLinks.map((link) => (
              <FilterChip
                key={`link-${link}`}
                label={`^${link}`}
                onRemove={() => {
                  setJournalLinks(journalLinks.filter((it) => it !== link));
                  setJournalPage(1);
                }}
              />
            ))}
            <Button variant="link" size="sm" className="h-10 px-1 text-link md:h-8" onClick={clearFilters}>
              {t('ledger.common.clear_filters')}
            </Button>
          </div>
        )}
      </div>

      {content}

      {data && <PagePagination page={journalPage} totalPages={data.total_page} onPageChange={onPage} />}
    </PageShell>
  );
}

export default Journals;

const REMOVE_ICON_CLASS = cn(
  'flex size-5 items-center justify-center rounded-full',
  'group-hover/remove:bg-foreground/10 group-focus-visible/remove:ring-2 group-focus-visible/remove:ring-ring',
);

/** Active filter chip; the remove button has a 40px hit area on mobile while the pill itself stays compact. */
function FilterChip({ label, onRemove }: { label: string; onRemove: () => void }) {
  const { t } = useTranslation();
  return (
    <span className="inline-flex h-8 items-center rounded-md bg-secondary pl-2.5 text-sm text-secondary-foreground md:h-6 md:text-xs">
      {label}
      <button
        type="button"
        className="group/remove -my-1 flex size-10 items-center justify-center rounded-full outline-none md:my-0 md:size-6"
        aria-label={t('ledger.common.remove_filter', { value: label })}
        onClick={onRemove}
      >
        <span className={REMOVE_ICON_CLASS}>
          <X className="size-3" />
        </span>
      </button>
    </span>
  );
}

/** Day heading parts: `Sep 16` + `Sat` (`9月16日` + `周六`); the year is added for dates outside the current year. */
function useDayHeading() {
  const fmt = useDateFormat();
  const thisYear = new Date().getFullYear();
  return (date: string) => {
    const day = parseISO(date);
    return { day: day.getFullYear() === thisYear ? fmt.day(day) : fmt.date(day), weekday: fmt.format(day, 'EEE') };
  };
}

const DAY_HEADING = cn(
  'sticky top-14 z-10 -mx-4 flex items-baseline gap-1.5 px-4 py-1 text-sm font-semibold md:static md:mx-0 md:px-0 md:py-0',
  'bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/80 md:bg-transparent md:backdrop-blur-none',
);

/** Journals grouped by day (多少记账 style): a heading per day, then one bordered card of `JournalRow`s. */
function JournalDays() {
  const groupedRecords = useAtomValue(groupedJournalsAtom);
  const heading = useDayHeading();
  return (
    <div className="flex flex-col gap-5">
      {Object.keys(groupedRecords).map((date) => {
        const { day, weekday } = heading(date);
        return (
          <section key={date} className="flex flex-col gap-2">
            <h2 className={DAY_HEADING}>
              {day}
              <span className="text-xs font-normal text-muted-foreground">{weekday}</span>
            </h2>
            <div className="divide-y overflow-hidden rounded-lg border bg-card">
              {groupedRecords[date].map((journal) => (
                <JournalRow key={journal.id} data={journal} />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
