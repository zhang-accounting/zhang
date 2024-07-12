import { Button, Checkbox, CloseButton, Container, Group, Input, Table } from '@mantine/core';
import { useDocumentTitle, useInputState, useLocalStorage } from '@mantine/hooks';
import AccountLine from '../components/AccountLine';
import { AccountStatus } from '../rest-model';
import { loadable_unwrap } from '../states';
import { accountAtom, accountFetcher } from '../states/account';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';
import { IconFilter } from '@tabler/icons-react';
import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { selectAtom } from 'jotai/utils';
import AccountTrie from '../utils/AccountTrie';
import { titleAtom } from '../states/basic';
import { useMemo } from 'react';

export default function Accounts() {
  const { t } = useTranslation();
  const [filterKeyword, setFilterKeyword] = useInputState('');
  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });

  const [accountTrie] = useAtom(
    useMemo(
      () =>
        selectAtom(accountAtom, (val) => {
          return loadable_unwrap(val, new AccountTrie(), (data) => {
            let trie = new AccountTrie();
            for (let account of data.filter((it) => (hideClosedAccount ? it.status === AccountStatus.Open : true))) {
              let trimmedKeyword = filterKeyword.trim();
              if (trimmedKeyword !== '') {
                if (
                  account.name.toLowerCase().includes(trimmedKeyword.toLowerCase()) ||
                  (account.alias?.toLowerCase() ?? '').includes(trimmedKeyword.toLowerCase())
                ) {
                  trie.insert(account);
                }
              } else {
                trie.insert(account);
              }
            }
            return trie;
          });
        }),
      [filterKeyword, hideClosedAccount],
    ),
  );

  const refreshAccounts = useSetAtom(accountFetcher);

  const ledgerTitle = useAtomValue(titleAtom);

  useDocumentTitle(`Accounts - ${ledgerTitle}`);

  return (
    <Container fluid>
      <Heading title={'Accounts'}></Heading>
      <Group my="lg">
        <Input
          leftSection={<IconFilter size="1rem" />}
          placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
          value={filterKeyword}
          onChange={setFilterKeyword}
          rightSection={<CloseButton aria-label={t('ACCOUNT_FILTER_CLOSE_BUTTON_ARIA')} onClick={() => setFilterKeyword('')} />}
        />
      </Group>
      <Group my="lg">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => refreshAccounts()}>
          {t('REFRESH')}
        </Button>
        <Checkbox checked={hideClosedAccount} onChange={() => setHideClosedAccount(!hideClosedAccount)} label={'Hide closed accounts'} />
      </Group>

      <Table verticalSpacing="xs" withTableBorder highlightOnHover>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Name</Table.Th>
            <Table.Th style={{ textAlign: 'end' }}>Balance</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {Object.keys(accountTrie.children)
            .sort()
            .map((item) => (
              <AccountLine spacing={0} key={accountTrie.children[item].path} data={accountTrie.children[item]} />
            ))}
        </Table.Tbody>
      </Table>
    </Container>
  );
}
