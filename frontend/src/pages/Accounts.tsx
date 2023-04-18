import { Chip, Container, Group, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect } from 'react';
import AccountLine from '../components/AccountLine';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchAccounts, getAccountsTrie } from '../states/account';

export default function Accounts() {
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
      <Title order={2}>Accounts</Title>

      <Group my="lg">
        <Chip checked={hideClosedAccount} onChange={() => setHideClosedAccount(!hideClosedAccount)}>
          Hide closed accounts
        </Chip>
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
