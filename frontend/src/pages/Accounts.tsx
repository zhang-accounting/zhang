import {Chip, Container, Group, Table, Title} from '@mantine/core';
import {useLocalStorage} from '@mantine/hooks';
import {useEffect, useState} from 'react';
import AccountLine from '../components/AccountLine';
import AccountTrie from '../utils/AccountTrie';
import useSWR from 'swr'
import {fetcher} from "../index";
import {Account, AccountStatus} from "../rest-model";

export default function Accounts() {
  const {data, error } =  useSWR<Account[]>("/api/accounts", fetcher);

  const [hideClosedAccount, setHideClosedAccount] = useLocalStorage({ key: 'hideClosedAccount', defaultValue: false });

  const [accountTrie, setAccountTrie] = useState(new AccountTrie());

  useEffect(() => {
    if (data) {
      let trie = new AccountTrie();
      for (let account of data.filter((it) => (hideClosedAccount ? it.status === AccountStatus.Open : true))) {
        trie.insert(account);
      }
      setAccountTrie(trie);
    }
  }, [data, hideClosedAccount]);

  if (error) return <div>failed to load</div>
  if (!data) return <div>loading...</div>
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
