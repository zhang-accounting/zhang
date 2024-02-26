import { Button, Checkbox, CloseButton, Container, Group, Input, Table } from '@mantine/core';
import { useInputState, useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import AccountLine from '../components/AccountLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchAccounts, getAccountsTrie } from '../states/account';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';
import { IconFilter } from '@tabler/icons';

export default function Accounts() {
  const { t } = useTranslation();
  const [filterKeyword, setFilterKeyword] = useInputState('');
  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });
  const dispatch = useAppDispatch();
  const accountStatus = useAppSelector((state) => state.accounts.status);
  const accountTrie = useAppSelector(getAccountsTrie(hideClosedAccount, filterKeyword));

  useEffect(() => {
    if (accountStatus === LoadingState.NotReady) {
      dispatch(fetchAccounts());
    }
  }, [dispatch, accountStatus]);

  return (
    <Container fluid>
      <Heading title={`Accounts`}></Heading>
      <Group my="lg">
        <Input
          icon={<IconFilter size="1rem" />}
          placeholder={t('ACCOUNT_FILTER_PLACEHOLDER')}
          value={filterKeyword}
          onChange={setFilterKeyword}
          rightSection={<CloseButton aria-label={t('ACCOUNT_FILTER_CLOSE_BUTTON_ARIA')} onClick={() => setFilterKeyword('')} />}
        />
      </Group>
      <Group my="lg">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => dispatch(fetchAccounts())}>
          {t('REFRESH')}
        </Button>
        <Checkbox checked={hideClosedAccount} onChange={() => setHideClosedAccount(!hideClosedAccount)} label={'Hide closed accounts'} />
      </Group>

      <Table verticalSpacing="xs" highlightOnHover withBorder>
        <thead>
          <tr>
            <th>Name</th>
            <th style={{ textAlign: 'end' }}>Balance</th>
          </tr>
        </thead>
        <tbody>
          {Object.keys(accountTrie.children)
            .sort()
            .map((item) => (
              <AccountLine spacing={0} key={accountTrie.children[item].path} data={accountTrie.children[item]} />
            ))}
        </tbody>
      </Table>
    </Container>
  );
}
