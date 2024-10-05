import { Container, Grid, Progress, Table, Tooltip } from '@mantine/core';
import { DatePickerInput } from '@mantine/dates';
import { IconCalendar } from '@tabler/icons-react';
import { useEffect, useState } from 'react';
import useSWR from 'swr';
import Amount from '../components/Amount';
import ReportGraph from '../components/ReportGraph';
import Section from '../components/Section';
import StatusGroup from '../components/StatusGroup';
import { StatisticGraphResponse, StatisticResponse, StatisticTypeResponse } from '../rest-model';
import PayeeNarration from '../components/basic/PayeeNarration';
import BigNumber from 'bignumber.js';
import { Heading } from '../components/basic/Heading';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { fetcher } from '../global.ts';

const color_set = ['pink', 'grape', 'violet'];

export default function Report() {
  const [value, setValue] = useState<[Date | null, Date | null]>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);

  const [dateRange, setDateRange] = useState<[Date, Date]>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Report - ${ledgerTitle}`);

  useEffect(() => {
    if (value[0] !== null && value[1] !== null) {
      setDateRange([value[0], value[1]]);
    }
  }, [value]);

  const { data, error } = useSWR<StatisticResponse>(
    `/api/statistic/summary?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}&interval=Day`,
    fetcher,
  );

  const { data: graph_data } = useSWR<StatisticGraphResponse>(
    `/api/statistic/graph?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}&interval=Day`,
    fetcher,
  );

  const { data: income_data } = useSWR<StatisticTypeResponse>(
    `/api/statistic/Income?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}`,
    fetcher,
  );

  const { data: expenses_data } = useSWR<StatisticTypeResponse>(
    `/api/statistic/Expenses?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}`,
    fetcher,
  );

  const income_total =
    income_data?.detail.reduce((acc, current) => acc.plus(new BigNumber(current.amount.calculated.number)), new BigNumber(0)).toNumber() ?? 0;
  const expense_total =
    expenses_data?.detail.reduce((acc, current) => acc.plus(new BigNumber(current.amount.calculated.number)), new BigNumber(0)).toNumber() ?? 0;

  // if (reportError) return <div>failed to load</div>;
  // if (!reportData) return <>loading</>;
  if (!graph_data) return <>loading</>;

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <>
      <Container fluid>
        <Heading
          title={`Report`}
          rightSection={
            <DatePickerInput
              leftSection={<IconCalendar size="1.1rem" stroke={1.5} />}
              clearable
              type="range"
              allowSingleDateInRange
              value={value}
              onChange={setValue}
            />
          }
        ></Heading>
        <StatusGroup
          data={[
            {
              title: '资产余额',
              amount: data.balance.calculated.number,
              currency: data.balance.calculated.currency,
            },
            // { title: '总收支', amount: data?.statistic.incomeExpense.summary.number, currency: data?.statistic.incomeExpense.summary.currency },
            {
              title: '收入',
              amount: data.income.calculated.number,
              currency: data.income.calculated.currency,
            },
            {
              title: '支出',
              amount: data.expense.calculated.number,
              currency: data.expense.calculated.currency,
            },
            { title: '交易数', number: data.transaction_number },
          ]}
        />

        <Section title="Graph">
          <ReportGraph data={graph_data} height={90}></ReportGraph>
        </Section>

        <Section title="Incomes">
          <Grid>
            <Grid.Col span={12}>
              <Progress.Root radius="sm" size={24}>
                {income_data?.detail.map((item, idx) => (
                  <Tooltip label={`${item.account} - ${item.amount.calculated.number}`}>
                    <Progress.Section
                      value={new BigNumber(item.amount.calculated.number).div(income_total).multipliedBy(100).toNumber()}
                      color={color_set[idx % color_set.length]}
                    >
                      <Progress.Label>{item.account}</Progress.Label>
                    </Progress.Section>
                  </Tooltip>
                ))}
              </Progress.Root>
            </Grid.Col>
            <Grid.Col span={12}>
              <Table verticalSpacing="xs" highlightOnHover>
                <Table.Thead>
                  <TableRow>
                    <Table.Th>Date</Table.Th>
                    <Table.Th>Account</Table.Th>
                    <Table.Th style={{}}>Payee & Narration</Table.Th>
                    <Table.Th>Amount</Table.Th>
                  </TableRow>
                </Table.Thead>
                <tbody>
                {income_data?.top_transactions.map((journal) => (
                  <TableRow>
                    <TableCell>{journal.datetime}</TableCell>
                    <TableCell>{journal.account}</TableCell>
                    <TableCell>
                      <PayeeNarration payee={journal.payee} narration={journal.narration} />
                    </TableCell>
                    <TableCell>
                      <Amount amount={journal.inferred_unit_number} currency={journal.inferred_unit_commodity} />
                    </TableCell>
                  </TableRow>
                ))}
                </tbody>
              </Table>
            </Grid.Col>
          </Grid>
        </Section>

        <Section title="Expenses">
          <Grid>
            <Grid.Col span={12}>
              <Progress.Root radius="sm" size={24}>
                {expenses_data?.detail.map((item, idx) => (
                  <Tooltip label={`${item.account} - ${item.amount.calculated.number}`}>
                    <Progress.Section
                      value={new BigNumber(item.amount.calculated.number).div(expense_total).multipliedBy(100).toNumber()}
                      color={color_set[idx % color_set.length]}
                    >
                      <Progress.Label>{item.account}</Progress.Label>
                    </Progress.Section>
                  </Tooltip>
                ))}
              </Progress.Root>
            </Grid.Col>
            <Grid.Col span={12}>
              <Table verticalSpacing="xs" highlightOnHover>
                <Table.Thead>
                  <TableRow>
                    <Table.Th>Date</Table.Th>
                    <Table.Th>Account</Table.Th>
                    <Table.Th style={{}}>Payee & Narration</Table.Th>
                    <Table.Th>Amount</Table.Th>
                  </TableRow>
                </Table.Thead>
                <tbody>
                {expenses_data?.top_transactions.map((journal) => (
                  // <JournalLine key={idx} data={journal} />
                  <TableRow>
                    <TableCell>{journal.datetime}</TableCell>
                    <TableCell>{journal.account}</TableCell>
                    <TableCell>
                      {journal.payee} {journal.narration}
                    </TableCell>
                    <TableCell>
                      <Amount amount={journal.inferred_unit_number} currency={journal.inferred_unit_commodity} />
                    </TableCell>
                  </TableRow>
                ))}
                </tbody>
              </Table>
            </Grid.Col>
          </Grid>
        </Section>
      </Container>
    </>
  );
}
