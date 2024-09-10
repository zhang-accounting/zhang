import { Badge, Button, CloseButton, Group, Input, Pagination, Pill, Table, Text } from '@mantine/core';
import { useEffect, useMemo, useState } from 'react';
import TableViewJournalLine from '../components/journalLines/tableView/TableViewJournalLine';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';
import { useDebouncedValue, useDocumentTitle } from '@mantine/hooks';
import { IconFilter } from '@tabler/icons-react';
import { JournalListSkeleton } from '../components/skeletons/journalListSkeleton';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { groupedJournalsAtom, journalAtom, journalFetcher, journalKeywordAtom, journalLinksAtom, journalPageAtom, journalTagsAtom } from '../states/journals';
import { useAtom, useSetAtom } from 'jotai';
import { loadable_unwrap } from '../states';
import { selectAtom } from 'jotai/utils';

function Journals() {
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

  const onPage = (page: number) => {
    setJournalPage(page);
  };

  return (
    <>
      <Heading title={`${total_count} Journals`}></Heading>
      <Group my="lg" px="sm">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => refreshJournals()}>
          {t('REFRESH')}
        </Button>
        <Input
          leftSection={<IconFilter size="1rem" />}
          placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
          value={filter}
          onChange={(event: any) => setFilter(event.currentTarget.value)}
          rightSection={<CloseButton aria-label={t('ACCOUNT_FILTER_CLOSE_BUTTON_ARIA')} onClick={() => setFilter('')} />}
        />
      </Group>
      <Group my="lg" px="sm">
        {journalTags.map((tag, index) => (
          <Pill key={index} withRemoveButton onRemove={() => removeTag(tag)}>
            #{tag}
          </Pill>
        ))}
        {journalLinks.map((link, index) => (
          <Pill key={index} withRemoveButton onRemove={() => removeLink(link)}>
            ^{link}
          </Pill>
        ))}
      </Group>
      <Table verticalSpacing="xs" withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th style={{ width: '100px' }}>Date</Table.Th>
            <Table.Th style={{ width: '10px' }}>Type</Table.Th>
            <Table.Th>Payee · Narration</Table.Th>
            <Table.Th style={{ textAlign: 'right' }}>Amount</Table.Th>
            <Table.Th style={{ textAlign: 'right' }}>Operation</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {(journalItems.state === 'loading' || journalItems.state === 'hasError') && <JournalListSkeleton />}
          {journalItems.state === 'hasData' &&
            Object.keys(groupedRecords).map((date) => {
              return (
                <>
                  <Table.Tr key={date}>
                    <Table.Td colSpan={6}>
                      <Text c={'dimmed'} size={'sm'}>
                        {date}
                      </Text>
                    </Table.Td>
                  </Table.Tr>
                  {groupedRecords[date].map((journal) => (
                    <TableViewJournalLine key={journal.id} data={journal} />
                  ))}
                </>
              );
            })}
        </Table.Tbody>
      </Table>

      <Group justify={'center'}>
        <Pagination my="xs" total={total_page} value={journalPage} onChange={onPage} />
      </Group>
    </>
  );
}

export default Journals;
