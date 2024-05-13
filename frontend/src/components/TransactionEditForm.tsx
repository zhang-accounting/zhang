import { ActionIcon, Code, Container, Divider, Grid, Select, Autocomplete, TextInput } from '@mantine/core';
import { DateInput } from '@mantine/dates';
import DividerWithAction from './basic/DividerWithAction';
import { IconTextPlus, IconTrashX } from '@tabler/icons-react';
import { useListState } from '@mantine/hooks';
import { useEffect, useState } from 'react';
import useSWR from 'swr';
import { InfoForNewTransaction, JournalTransactionItem } from '../rest-model';
import { fetcher } from '../index';
import { useTranslation } from 'react-i18next';
import { format } from 'date-fns';
import { useAppSelector } from '../states';
import { getAccountSelectItems } from '../states/account';

interface Posting {
  account: string | null;
  amount: string;
}

interface SelectItem {
  label: string;
  value: string;
  group?: string;
}

interface Props {
  onChange(data: any, isValid: boolean): void;

  data?: JournalTransactionItem;
}

export default function TransactionEditForm(props: Props) {
  const { t } = useTranslation();

  const { data, error } = useSWR<InfoForNewTransaction>('/api/for-new-transaction', fetcher);

  const [dateOnly] = useState(true);
  const [date, setDate] = useState<Date | null>(props.data?.datetime ? new Date(props.data?.datetime) : new Date());
  const [payee, setPayee] = useState<string | undefined>(props.data?.payee ?? undefined);
  const [narration, setNarration] = useState(props.data?.narration ?? '');
  const [postings, postingsHandler] = useListState<Posting>(
    props.data?.postings?.map((item) => ({
      account: item.account,
      amount: `${item.unit_number ?? ''} ${item.unit_commodity ?? ''}`.trim(),
    })) ?? [
      { account: null, amount: '' },
      { account: null, amount: '' },
    ],
  );

  const [metas, metaHandler] = useListState<{ key: string; value: string }>(props.data?.metas ?? []);

  const [payeeSelectItems, setPayeeSelectItems] = useState<SelectItem[]>([]);
  const accountItems = [...useAppSelector(getAccountSelectItems())];

  useEffect(() => {
    const newPayeeSelectItems: SelectItem[] = (data?.payee ?? []).map((item) => {
      return {
        label: item,
        value: item,
      };
    });
    setPayeeSelectItems(newPayeeSelectItems);
  }, [data, setPayeeSelectItems]);

  useEffect(() => {
    props.onChange(
      {
        datetime: date?.toISOString(),
        payee: payee ?? '',
        narration: narration,
        postings: postings.map((it) => ({
          account: it.account,
          unit:
            it.amount.trim() === ''
              ? null
              : {
                  number: it.amount.split(' ')[0],
                  commodity: it.amount.split(' ')[1],
                },
        })),
        tags: [],
        links: [],
        metas: metas,
      },
      valid(),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [date, payee, narration, postings, metas]);

  const preview = (): string => {
    const dateDisplay = format(date || 0, dateOnly ? 'yyyy-MM-dd' : 'yyyy-MM-dd HH:mm:ss');
    const narrationDisplay = narration.trim().length === 0 ? '' : ` ${JSON.stringify(narration.trim())}`;
    const postingDisplay = postings.map((posting) => `  ${posting.account} ${posting.amount}`).join('\n');
    const metaDisplay = metas
      .filter((meta) => meta.key.trim() !== '' && meta.value.trim() !== '')
      .map((meta) => `  ${JSON.stringify(meta.key)}: ${JSON.stringify(meta.value)}`)
      .join('\n');
    return `${dateDisplay} ${JSON.stringify(payee || '')}${narrationDisplay}\n${postingDisplay}\n${metaDisplay}`;
  };

  const valid = (): boolean => {
    return postings.every((posting) => posting.account !== null) && postings.filter((posting) => posting.amount.trim().length === 0).length <= 1;
  };

  const onPayeeCreate = (query: string) => {
    const newPayee = { label: query, value: query };
    setPayeeSelectItems([...payeeSelectItems, newPayee]);
    return newPayee;
  };

  if (error) return <div>failed to load</div>;
  if (!data) return <div>loading...</div>;
  return (
    <Container>
      <Grid>
        <Grid.Col span={{ lg: 4, sm: 12 }}>
          <DateInput firstDayOfWeek={0} placeholder="Transaction Date" value={date} onChange={setDate} withAsterisk />
        </Grid.Col>
        <Grid.Col span={{ lg: 4, sm: 12 }}>
          <Autocomplete
            placeholder="Payee"
            data={payeeSelectItems}
            value={payee}
            // todo getCreateLabel={(query) => `${t('NEW_TRANSACTION_PAYEE_CREATE')} ${query}`}
            // todo onCreate={onPayeeCreate}
            onChange={setPayee}
          />
        </Grid.Col>
        <Grid.Col span={{ lg: 4, sm: 12 }}>
          <TextInput placeholder="Narration" value={narration} onChange={(e) => setNarration(e.target.value)} />
        </Grid.Col>
      </Grid>

      <DividerWithAction
        value="Postings"
        icon={<IconTextPlus />}
        onActionClick={() =>
          postingsHandler.append({
            account: null,
            amount: '',
          })
        }
      ></DividerWithAction>

      {postings.map((posting, idx) => (
        <Grid align="center" key={idx}>
          <Grid.Col span={8}>
            <Select
              searchable
              placeholder="Account"
              data={accountItems}
              value={posting.account}
              onChange={(e) => postingsHandler.setItemProp(idx, 'account', e)}
            />
          </Grid.Col>
          <Grid.Col span={3}>
            <TextInput placeholder="Amount" value={posting.amount} onChange={(e) => postingsHandler.setItemProp(idx, 'amount', e.target.value)} />
          </Grid.Col>
          <Grid.Col span={1}>
            <ActionIcon variant="white" color="gray" disabled={postings.length <= 2} onClick={() => postingsHandler.remove(idx)}>
              <IconTrashX />
            </ActionIcon>
          </Grid.Col>
        </Grid>
      ))}

      <DividerWithAction
        value="Metas"
        icon={<IconTextPlus />}
        onActionClick={() => {
          metaHandler.append({ key: '', value: '' });
        }}
      ></DividerWithAction>

      {metas.map((meta, idx) => (
        <Grid align="center" key={idx}>
          <Grid.Col span={4}>
            <TextInput placeholder="key" value={meta.key} onChange={(e) => metaHandler.setItemProp(idx, 'key', e.target.value)} />
          </Grid.Col>
          <Grid.Col span={7}>
            <TextInput placeholder="value" value={meta.value} onChange={(e) => metaHandler.setItemProp(idx, 'value', e.target.value)} />
          </Grid.Col>
          <Grid.Col span={1}>
            <ActionIcon variant="white" color="gray" onClick={() => metaHandler.remove(idx)}>
              <IconTrashX />
            </ActionIcon>
          </Grid.Col>
        </Grid>
      ))}
      <Divider label="Preview" size="xs" my="md"></Divider>
      <Code block>{preview()}</Code>
    </Container>
  );
}
