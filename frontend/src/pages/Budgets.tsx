import { ActionIcon, Button, Chip, Container, Group, Popover, Table, Title } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { useState } from 'react';
import { BudgetListItem } from '../rest-model';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { fetcher } from '../index';
import { groupBy, sortBy } from 'lodash-es';
import BudgetCategory from '../components/budget/BudgetCategory';
import { format } from 'date-fns';
import { MonthPicker } from '@mantine/dates';
import { IconChevronLeft, IconChevronRight } from '@tabler/icons';

export default function Budgets() {
  const { t } = useTranslation();
  const [hideZeroAssignBudget, setHideZeroAssignBudget] = useLocalStorage({
    key: 'hideZeroAssignBudget',
    defaultValue: false,
  });

  const [date, setDate] = useState<Date>(new Date());

  const { data: budgets, error } = useSWR<BudgetListItem[]>(`/api/budgets?year=${date.getFullYear()}&month=${date.getMonth() + 1}`, fetcher);
  if (error) return <div>failed to load</div>;
  if (!budgets) return <div>loading...</div>;

  const goToMonth = (gap: number) => {
    let newDate = new Date(date);
    newDate.setMonth(newDate.getMonth() + gap);
    setDate(newDate);
  };

  return (
    <Container fluid>
      <Group>
        <ActionIcon onClick={() => goToMonth(-1)}>
          <IconChevronLeft size="1.125rem" />
        </ActionIcon>
        <Popover position="bottom" withArrow shadow="md">
          <Popover.Target>
            <Title style={{ display: 'inline-block', cursor: 'pointer' }} order={2} py="md" px="xs">{`${format(date, 'MMM, yyyy')}`}</Title>
          </Popover.Target>
          <Popover.Dropdown>
            <MonthPicker value={date} maxDate={new Date()} onChange={(newDate) => setDate(newDate ?? new Date())} />
          </Popover.Dropdown>
        </Popover>
        <ActionIcon onClick={() => goToMonth(1)} disabled={date.getFullYear() === new Date().getFullYear() && date.getMonth() === new Date().getMonth()}>
          <IconChevronRight size="1.125rem" />
        </ActionIcon>
      </Group>

      <Group my="lg">
        <Button variant="outline" color="gray" radius="xl" size="xs">
          {t('REFRESH')}
        </Button>
        <Chip checked={hideZeroAssignBudget} onChange={() => setHideZeroAssignBudget(!hideZeroAssignBudget)}>
          Hide Zero Amount Assigned Budget
        </Chip>
      </Group>
      <Table verticalSpacing="xs" withBorder>
        <thead>
          <tr>
            <th>Category</th>
            <th style={{ textAlign: 'end' }}>Assigned</th>
            <th style={{ textAlign: 'end' }}>Activity</th>
            <th style={{ textAlign: 'end' }}>Available</th>
          </tr>
        </thead>
        <tbody>
          {sortBy(Object.entries(groupBy(budgets, (budget) => budget.category)), (entry) => entry[0]).map((entry) => (
            <BudgetCategory key={`${entry[0]}-${date.getFullYear()}-${date.getMonth()}`} name={entry[0]} items={entry[1]}></BudgetCategory>
          ))}
        </tbody>
      </Table>
    </Container>
  );
}
