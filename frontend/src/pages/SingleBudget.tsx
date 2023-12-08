import { ActionIcon, Badge, Code, Container, createStyles, Group, Popover, px, Stack, Table, Tabs, Text, Title } from '@mantine/core';
import { IconChevronLeft, IconChevronRight, IconMessageCircle, IconPhoto, IconSettings } from '@tabler/icons';
import { format, parse, parseISO } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '..';
import AccountBalanceCheckLine from '../components/AccountBalanceCheckLine';
import AccountDocumentUpload from '../components/AccountDocumentUpload';
import Amount from '../components/Amount';
import LoadingComponent from '../components/basic/LoadingComponent';
import PayeeNarration from '../components/basic/PayeeNarration';
import AccountDocumentLine from '../components/documentLines/AccountDocumentLine';
import { AccountInfo, AccountJournalItem, BudgetInfoResponse, BudgetIntervalEventResponse, Document } from '../rest-model';
import { MonthPicker } from '@mantine/dates';
import { useState } from 'react';
import { groupBy, sortBy } from 'lodash';
import BudgetCategory from '../components/budget/BudgetCategory';
import dayjs from 'dayjs';
import moment from 'moment';

const useStyles = createStyles((theme) => ({
  calculatedAmount: {
    fontSize: px(theme.fontSizes.xl) * 1.1,
    fontWeight: 500,
  },
  detailAmount: {
    fontSize: px(theme.fontSizes.lg),
  },
}));

function SingleBudget() {
  let { budgetName } = useParams();
  const [date, setDate] = useState<Date>(new Date());

  const goToMonth = (gap: number) => {
    let newDate = new Date(date);
    newDate.setMonth(newDate.getMonth() + gap);
    setDate(newDate);
  };
  const { data: budget_info, error } = useSWR<BudgetInfoResponse>(`/api/budgets/${budgetName}`, fetcher);
  const { data: budget_interval_event, error: event_error } = useSWR<BudgetIntervalEventResponse[]>(
    `/api/budgets/${budgetName}/interval/${date.getFullYear()}/${date.getMonth() + 1}`,
    fetcher,
  );

  if (error) return <div>failed to load</div>;
  if (!budget_info) return <div>{error}</div>;
  return (
    <Container fluid>
      <Group position="apart" py="md" px="sm" align="center">
        <Stack>
          <Group>
            <Title order={2}>{budget_info.alias ?? budget_info.name}</Title>
            {budget_info.alias && <Code>{budget_info.name}</Code>}
          </Group>
          <Group>
            {budget_info.related_accounts.map((account) => (
              <Badge key={account} color="pink" variant="filled">
                {account}
              </Badge>
            ))}
          </Group>
        </Stack>
        <Group spacing={'lg'}>
          <Stack spacing="xs" align={'end'}>
            <Text size="xs" color={'dimmed'}>
              Assigned Amount
            </Text>
            <Amount amount={budget_info.assigned_amount.number} currency={budget_info.assigned_amount.currency}></Amount>
          </Stack>
          <Stack spacing="xs" align={'end'}>
            <Text size="xs" color={'dimmed'}>
              Activity Amount
            </Text>
            <Amount amount={budget_info.activity_amount.number} currency={budget_info.activity_amount.currency}></Amount>
          </Stack>
          <Stack spacing="xs" align={'end'}>
            <Text size="xs" color={'dimmed'}>
              Available Amount
            </Text>
            <Amount amount={budget_info.available_amount.number} currency={budget_info.available_amount.currency}></Amount>
          </Stack>
        </Group>
      </Group>

      <Group>
        <ActionIcon onClick={() => goToMonth(-1)}>
          <IconChevronLeft size="1rem" />
        </ActionIcon>
        <Popover position="bottom" withArrow shadow="md">
          <Popover.Target>
            <Text style={{ display: 'inline-block', cursor: 'pointer' }} py="md" px="xs">{`${format(date, 'MMM, yyyy')}`}</Text>
          </Popover.Target>
          <Popover.Dropdown>
            <MonthPicker value={date} maxDate={new Date()} onChange={(newDate) => setDate(newDate ?? new Date())} />
          </Popover.Dropdown>
        </Popover>
        <ActionIcon onClick={() => goToMonth(1)} disabled={date.getFullYear() == new Date().getFullYear() && date.getMonth() == new Date().getMonth()}>
          <IconChevronRight size="1rem" />
        </ActionIcon>
      </Group>

      <Table verticalSpacing="xs" withBorder>
        <thead>
          <tr>
            <th>Date</th>
            <th>Activity</th>
            <th>Account</th>
            <th style={{ textAlign: 'end' }}>Assigned Amount</th>
            <th style={{ textAlign: 'end' }}>Activity Amount</th>
          </tr>
        </thead>
        <tbody>
          {(budget_interval_event ?? []).map((it) => {
            return (
              <tr>
                <td>{format(it.timestamp * 1000, 'MMM dd HH:mm:ss')}</td>
                <td>{'event_type' in it ? it.event_type : <PayeeNarration payee={it.payee} narration={it.narration} />}</td>
                <td>
                  {!('event_type' in it) && (
                    <Badge color="pink" variant="filled">
                      {it.account}
                    </Badge>
                  )}
                </td>
                <td style={{ textAlign: 'end' }}>{'event_type' in it && <Amount amount={it.amount.number} currency={it.amount.currency} />}</td>
                <td style={{ textAlign: 'end' }}>
                  {!('event_type' in it) && <Amount amount={it.inferred_unit_number} currency={it.inferred_unit_commodity} />}
                </td>
              </tr>
            );
          })}

          {/*{sortBy(Object.entries(groupBy(budgets, (budget) => budget.category)), (entry) => entry[0]).map((entry) => (*/}
          {/*    <BudgetCategory key={`${entry[0]}-${date.getFullYear()}-${date.getMonth()}`} name={entry[0]} items={entry[1]}></BudgetCategory>*/}
          {/*))}*/}
        </tbody>
      </Table>
    </Container>
  );
}

export default SingleBudget;
