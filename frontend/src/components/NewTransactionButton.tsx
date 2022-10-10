import { gql, useMutation, useQuery } from '@apollo/client';
import { useDisclosure } from '@mantine/hooks';
import { format } from 'date-fns';
import _ from 'lodash';
import { useState } from 'react';
// @ts-ignore
import { ActionIcon, Button, Code, Container, Divider, Grid, Group, Modal, Select, TextInput } from '@mantine/core';
import { DatePicker } from '@mantine/dates';
import { IconSquarePlus, IconTextPlus, IconTrashX } from '@tabler/icons';
import { AccountItem } from '../gql/accountList';
interface Posting {
  account: string | null;
  amount: string;
}

interface SelectItem {
  label: string;
  value: string;
  group?: string;
}

export default function NewTransactionButton() {
  const newTransactionMetaData = useQuery<{
    accounts: AccountItem[];
    journals: { data: { payee: string }[] };
  }>(gql`
    query NEW_TRANSACTION_MODAL_DATA {
      accounts {
        name
      }
      journals(page: 1, size: 999999999) {
        data {
          ... on TransactionDto {
            payee
          }
        }
      }
    }
  `);

  const [appendData] = useMutation(
    gql`
      mutation APPEND_DATA($date: Int, $content: String) {
        appendData(date: $date, content: $content)
      }
    `,
    {
      refetchQueries: ['FILE_LIST', 'SINGLE_FILE_ENTRY', 'JOURNAL_LIST', 'BAR_STATISTIC'],
    },
  );

  const [isOpen, isOpenHandler] = useDisclosure(false);
  const [dateOnly] = useState(true);

  const [date, setDate] = useState<Date | null>(new Date());
  const [payee, setPayee] = useState<string | null>(null);
  const [narration, setNarration] = useState('');
  const [postings, setPostings] = useState<Posting[]>([
    { account: null, amount: '' },
    { account: null, amount: '' },
  ]);

  const updatePostingAccount = (idx: number, account: string | null) => {
    const clonedPostings = [...postings];
    clonedPostings[idx].account = account;
    setPostings(clonedPostings);
  };
  const updatePostingAmount = (idx: number, amount: string) => {
    const clonedPostings = [...postings];
    clonedPostings[idx].amount = amount;
    setPostings(clonedPostings);
  };

  const preview = (): string => {
    const dateDisplay = format(date || 0, dateOnly ? 'yyyy-MM-dd' : 'yyyy-MM-dd HH:mm:ss');
    const narrationDisplay = narration.trim().length === 0 ? '' : ` ${JSON.stringify(narration.trim())}`;
    const postingDisplay = postings.map((posting) => `  ${posting.account} ${posting.amount}`).join('\n');
    return `${dateDisplay} ${JSON.stringify(payee || '')}${narrationDisplay}\n${postingDisplay}`;
  };

  const valid = (): boolean => {
    return postings.every((posting) => posting.account !== null) && postings.filter((posting) => posting.amount.trim().length === 0).length <= 1;
  };
  const newPosting = () => {
    const newPostings = [...postings];
    newPostings.push({ account: null, amount: '' });
    setPostings(newPostings);
  };

  const handleDeletePosting = (targetIdx: number) => {
    setPostings(postings.filter((_, idx) => idx !== targetIdx));
  };
  const save = () => {
    appendData({
      variables: {
        date: Math.round((date?.getTime() || 0) / 1000),
        content: `\n${preview()}\n`,
      },
    });
    isOpenHandler.close();
    setDate(new Date());
    setPayee(null);
    setNarration('');
    setPostings([
      { account: null, amount: '' },
      { account: null, amount: '' },
    ]);
  };

  if (newTransactionMetaData.loading) return <p>Loading...</p>;
  if (newTransactionMetaData.error) return <p>Error :(</p>;

  const payeeSelectItems: SelectItem[] = _.uniqBy(
    _.filter(newTransactionMetaData.data!.journals.data, (journal) => !_.isEmpty(journal.payee)),
    (journal) => journal.payee,
  ).map((journal) => {
    return {
      label: journal.payee,
      value: journal.payee,
    };
  });

  const accountItems = (newTransactionMetaData.data?.accounts || []).map((singleAccountInfo) => {
    const type = singleAccountInfo.name.split(':')[0];
    return {
      label: singleAccountInfo.name,
      value: singleAccountInfo.name,
      group: type,
    };
  });

  return (
    <>
      <Button size="xs" leftIcon={<IconSquarePlus />} onClick={() => isOpenHandler.open()}>
        NEW
      </Button>

      <Modal onClose={() => isOpenHandler.close()} opened={isOpen} size="xl" centered closeOnEscape overflow="inside" title="New Transaction">
        <Container>
          <Group grow>
            <DatePicker placeholder="Pick date" value={date} onChange={setDate} withAsterisk />
            <Select
              placeholder="Pick one"
              data={payeeSelectItems}
              value={payee}
              searchable
              creatable
              getCreateLabel={(query) => `+ Create ${query}`}
              onChange={setPayee}
            />
            <TextInput placeholder="Narration" value={narration} onChange={(e) => setNarration(e.target.value)} />
          </Group>

          <Divider label="Postings" size="xs" my="md"></Divider>
          {postings.map((posting, idx) => (
            <Grid align="center">
              <Grid.Col span={8}>
                <Select searchable placeholder="Pick one" data={accountItems} value={posting.account} onChange={(e) => updatePostingAccount(idx, e)} />
              </Grid.Col>
              <Grid.Col span={3}>
                <TextInput placeholder="Input description" value={posting.amount} onChange={(e) => updatePostingAmount(idx, e.target.value)} />
              </Grid.Col>
              <Grid.Col span={1}>
                <ActionIcon disabled={postings.length <= 2} onClick={() => handleDeletePosting(idx)}>
                  <IconTrashX />
                </ActionIcon>
              </Grid.Col>
            </Grid>
          ))}

          <Group position="left" my="sm">
            <Button size="xs" leftIcon={<IconTextPlus />} variant="outline" onClick={newPosting}>
              new posting
            </Button>
          </Group>

          <Divider label="Preview" size="xs" my="md"></Divider>
          <Code block>{preview()}</Code>

          <Group position="right" my="md">
            <Button variant="outline" onClick={isOpenHandler.close}>
              Cancel
            </Button>
            <Button mr={3} onClick={save} disabled={!valid()}>
              Save
            </Button>
          </Group>
        </Container>
      </Modal>
    </>
  );
}
