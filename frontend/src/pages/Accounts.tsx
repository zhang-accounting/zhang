import { useQuery } from '@apollo/client';
import { Chip, Container, Group, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useEffect, useState } from 'react';
import AccountLine from '../components/AccountLine';
import { AccountListQuery, ACCOUNT_LIST } from '../gql/accountList';
import AccountTrie from '../utils/AccountTrie';

export default function Accounts() {
  const { loading, error, data } = useQuery<AccountListQuery>(ACCOUNT_LIST);

  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });

  const [accountTrie, setAccountTrie] = useState(new AccountTrie());

  useEffect(() => {
    if (data) {
      let trie = new AccountTrie();
      for (let account of data.accounts.filter((it) => (hideClosedAccount ? it.status === 'OPEN' : true))) {
        trie.insert(account);
      }
      setAccountTrie(trie);
    }
  }, [data, hideClosedAccount]);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;
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
            <th>Balance</th>
          </tr>
        </thead>
        <tbody>
          {Object.keys(accountTrie.children)
            .sort()
            .map((item) => (
              <AccountLine spacing={0} key={accountTrie.children[item].path} data={accountTrie.children[item]} />
            ))}
          {/* </> */}
          {/* ))} */}
        </tbody>
      </Table>
    </Container>
  );
}
