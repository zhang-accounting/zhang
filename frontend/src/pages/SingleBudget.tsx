import { ActionIcon, Badge, Code, Container, Group, Popover, Stack, Table, Text, Title } from '@mantine/core';
import { IconChevronLeft, IconChevronRight } from '@tabler/icons-react';
import { format } from 'date-fns';
import { useParams } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '../global.ts';
import Amount from '../components/Amount';
import PayeeNarration from '../components/basic/PayeeNarration';
import { BudgetInfoResponse, BudgetIntervalEventResponse } from '../rest-model';
import { MonthPicker } from '@mantine/dates';
import { useState } from 'react';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';

function SingleBudget() {
  let { budgetName } = useParams();
  const [date, setDate] = useState<Date>(new Date());
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${budgetName} | Budgets - ${ledgerTitle}`);

  const goToMonth = (gap: number) => {
    let newDate = new Date(date);
    newDate.setMonth(newDate.getMonth() + gap);
    setDate(newDate);
  };
  const { data: budget_info, error } = useSWR<BudgetInfoResponse>(`/api/budgets/${budgetName}`, fetcher);
  const { data: budget_interval_event } = useSWR<BudgetIntervalEventResponse[]>(
    `/api/budgets/${budgetName}/interval/${date.getFullYear()}/${date.getMonth() + 1}`,
    fetcher,
  );

  if (error) return <div>failed to load</div>;
  if (!budget_info) return <div>{error}</div>;
  return (
    <Container fluid>
      <Group justify="space-between" py="md" px="sm" align="center">
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
        <Group gap={'lg'}>
          <Stack gap="xs" align={'end'}>
            <Text size="xs" c={'dimmed'}>
              Assigned Amount
            </Text>
            <Amount amount={budget_info.assigned_amount.number}
                    currency={budget_info.assigned_amount.currency}></Amount>
          </Stack>
          <Stack gap="xs" align={'end'}>
            <Text size="xs" c={'dimmed'}>
              Activity Amount
            </Text>
            <Amount amount={budget_info.activity_amount.number}
                    currency={budget_info.activity_amount.currency}></Amount>
          </Stack>
          <Stack gap="xs" align={'end'}>
            <Text size="xs" c={'dimmed'}>
              Available Amount
            </Text>
            <Amount amount={budget_info.available_amount.number}
                    currency={budget_info.available_amount.currency}></Amount>
          </Stack>
        </Group>
      </Group>

      <Group>
        <ActionIcon variant="white" onClick={() => goToMonth(-1)}>
          <IconChevronLeft size="1rem" />
        </ActionIcon>
        <Popover position="bottom" withArrow shadow="md">
          <Popover.Target>
            <Text style={{ display: 'inline-block', cursor: 'pointer' }} py="md"
                  px="xs">{`${format(date, 'MMM, yyyy')}`}</Text>
          </Popover.Target>
          <Popover.Dropdown>
            <MonthPicker value={date} maxDate={new Date()} onChange={(newDate) => setDate(newDate ?? new Date())} />
          </Popover.Dropdown>
        </Popover>
        <ActionIcon
          variant="white"
          onClick={() => goToMonth(1)}
          disabled={date.getFullYear() === new Date().getFullYear() && date.getMonth() === new Date().getMonth()}
        >
          <IconChevronRight size="1rem" />
        </ActionIcon>
      </Group>

      <Table verticalSpacing="xs" withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Date</Table.Th>
            <Table.Th>Activity</Table.Th>
            <Table.Th>Account</Table.Th>
            <Table.Th style={{ textAlign: 'end' }}>Assigned Amount</Table.Th>
            <Table.Th style={{ textAlign: 'end' }}>Activity Amount</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <tbody>
        {(budget_interval_event ?? []).map((it) => {
          return (
            <Table.Tr>
              <Table.Td>{format(it.timestamp * 1000, 'MMM dd HH:mm:ss')}</Table.Td>
              <Table.Td>{'event_type' in it ? it.event_type :
                <PayeeNarration payee={it.payee} narration={it.narration} />}</Table.Td>
              <Table.Td>
                {!('event_type' in it) && (
                  <Badge color="pink" variant="filled">
                    {it.account}
                  </Badge>
                )}
              </Table.Td>
              <Table.Td style={{ textAlign: 'end' }}>{'event_type' in it &&
                <Amount amount={it.amount.number} currency={it.amount.currency} />}</Table.Td>
              <Table.Td style={{ textAlign: 'end' }}>
                {!('event_type' in it) &&
                  <Amount amount={it.inferred_unit_number} currency={it.inferred_unit_commodity} />}
              </Table.Td>
            </Table.Tr>
          );
        })}
        </tbody>
      </Table>
    </Container>
  );
}

export default SingleBudget;
