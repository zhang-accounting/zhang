import { Button, Chip, Container, Group, Table, Checkbox } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import AccountLine from '../components/AccountLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchAccounts, getAccountsTrie } from '../states/account';
import { Heading } from '../components/basic/Heading';
import { useTranslation } from 'react-i18next';

export default function Accounts() {
  const { t } = useTranslation();
  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });
  const dispatch = useAppDispatch();
  const accountStatus = useAppSelector((state) => state.accounts.status);
  const accountTrie = useAppSelector(getAccountsTrie(hideClosedAccount));

  useEffect(() => {
    if (accountStatus === LoadingState.NotReady) {
      dispatch(fetchAccounts());
    }
  }, [dispatch, accountStatus]);

  return (
    <Container fluid>
      <Heading title={`Accounts`}></Heading>
      <Group my="lg">
        <Button variant="outline" color="gray" radius="xl" size="xs" onClick={() => dispatch(fetchAccounts())}>
          {t('REFRESH')}
        </Button>
        <Checkbox checked={hideClosedAccount} onChange={() => setHideClosedAccount(!hideClosedAccount)} label={'Hide closed accounts'} />
      </Group>
      <Table verticalSpacing="xs" highlightOnHover>
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
