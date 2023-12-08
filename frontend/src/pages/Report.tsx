import { Container, Grid, Progress, Table } from '@mantine/core';
import { DatePickerInput } from '@mantine/dates';
import { IconCalendar } from '@tabler/icons';
import { useEffect, useState } from 'react';
import useSWR from 'swr';
import Amount from '../components/Amount';
import ReportGraph from '../components/ReportGraph';
import Section from '../components/Section';
import StatusGroup from '../components/StatusGroup';
import { fetcher } from '../index';
import { StatisticGraphResponse, StatisticResponse, StatisticTypeResponse } from '../rest-model';
import PayeeNarration from '../components/basic/PayeeNarration';
import BigNumber from 'bignumber.js';
import { Heading } from '../components/basic/Heading';

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

  useEffect(() => {
    console.log('value is ', value);
    if (value[0] !== null && value[1] !== null) {
      console.log('update value', value);
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
              icon={<IconCalendar size="1.1rem" stroke={1.5} />}
              clearable
              placeholder="Pick dates range"
              type="range"
              allowSingleDateInRange
              value={value}
              onChange={setValue}
            />
          }
        ></Heading>
        <StatusGroup
          data={[
            { title: '资产余额', amount: data.balance.calculated.number, currency: data.balance.calculated.currency },
            // { title: '总收支', amount: data?.statistic.incomeExpense.summary.number, currency: data?.statistic.incomeExpense.summary.currency },
            { title: '收入', amount: data.income.calculated.number, currency: data.income.calculated.currency },
            { title: '支出', amount: data.expense.calculated.number, currency: data.expense.calculated.currency },
            { title: '交易数', number: data.transaction_number },
          ]}
        />

        <Section title="Graph">
          <ReportGraph data={graph_data} height={90}></ReportGraph>
        </Section>

        <Section title="Incomes">
          <Grid>
            <Grid.Col span={12}>
              <Progress
                radius="sm"
                size={24}
                sections={income_data?.detail.map((item, idx) => ({
                  value: new BigNumber(item.amount.calculated.number).div(income_total).multipliedBy(100).toNumber(),
                  color: color_set[idx % color_set.length],
                  label: item.account,
                  tooltip: `${item.account} - ${item.amount.calculated.number}`,
                }))}
              />
            </Grid.Col>
            <Grid.Col span={12}>
              <Table verticalSpacing="xs" highlightOnHover>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th>Account</th>
                    <th style={{}}>Payee & Narration</th>
                    <th>Amount</th>
                  </tr>
                </thead>
                <tbody>
                  {income_data?.top_transactions.map((journal) => (
                    <tr>
                      <td>{journal.datetime}</td>
                      <td>{journal.account}</td>
                      <td>
                        <PayeeNarration payee={journal.payee} narration={journal.narration} />
                      </td>
                      <td>
                        <Amount amount={journal.inferred_unit_number} currency={journal.inferred_unit_commodity} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            </Grid.Col>
          </Grid>
        </Section>

        <Section title="Expenses">
          <Grid>
            <Grid.Col span={12}>
              <Progress
                radius="sm"
                size={24}
                sections={expenses_data?.detail.map((item, idx) => ({
                  value: new BigNumber(item.amount.calculated.number).div(expense_total).multipliedBy(100).toNumber(),
                  color: color_set[idx % color_set.length],
                  label: item.account,
                  tooltip: `${item.account} - ${item.amount.calculated.number}`,
                }))}
              />
            </Grid.Col>
            <Grid.Col span={12}>
              <Table verticalSpacing="xs" highlightOnHover>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th>Account</th>
                    <th style={{}}>Payee & Narration</th>
                    <th>Amount</th>
                  </tr>
                </thead>
                <tbody>
                  {expenses_data?.top_transactions.map((journal) => (
                    // <JournalLine key={idx} data={journal} />
                    <tr>
                      <td>{journal.datetime}</td>
                      <td>{journal.account}</td>
                      <td>
                        {journal.payee} {journal.narration}
                      </td>
                      <td>
                        <Amount amount={journal.inferred_unit_number} currency={journal.inferred_unit_commodity} />
                      </td>
                    </tr>
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
