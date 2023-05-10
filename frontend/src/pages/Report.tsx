import { Box, Container, Grid, Group, Progress, SegmentedControl, Table, Text, Title } from '@mantine/core';
import { DateRangePicker, DateRangePickerValue } from '@mantine/dates';
import { useEffect, useState } from 'react';
import Section from '../components/Section';
import StatusGroup from '../components/StatusGroup';
import useSWR from 'swr';
import { fetcher } from '../index';
import { ReportResponse, StatisticResponse } from '../rest-model';
import Amount from '../components/Amount';
import ReportGraph from '../components/ReportGraph';


export default function Report() {
  const [value, setValue] = useState<DateRangePickerValue>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);

  const [dateRange, setDateRange] = useState<[Date, Date]>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);

  useEffect(() => {
    if (value[0] !== null && value[1] !== null) {
      console.log('update value', value);
      setDateRange([value[0], value[1]]);
    }
  }, [value]);

  const [gap, setGap] = useState('Day');

  const { data, error } = useSWR<StatisticResponse>(
    `/api/statistic?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}&interval=${gap}`,
    fetcher,
  );

  const { data: reportData, error: reportError } = useSWR<ReportResponse>(
    `/api/report?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}`,
    fetcher,
  );

  if (reportError) return <div>failed to load</div>;
  if (!reportData) return <>loading</>;

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <>
      <Container fluid>
        <Group position="apart" my="lg">
          <Title order={2}>Report</Title>
          <DateRangePicker placeholder="Pick dates range" value={value} onChange={setValue} />
        </Group>

        <StatusGroup
          data={[
            { title: '资产余额', amount: reportData.balance.number, currency: reportData.balance.commodity },
            // { title: '总收支', amount: data?.statistic.incomeExpense.summary.number, currency: data?.statistic.incomeExpense.summary.currency },
            { title: '收入', amount: reportData.income.number, currency: reportData.income.commodity },
            { title: '支出', amount: reportData.expense.number, currency: reportData.expense.commodity },
            { title: '交易数', number: reportData.transaction_number },
          ]}
        />

        <Section
          title="Graph"
          rightSection={
            <SegmentedControl
              size="xs"
              value={gap.toString()}
              onChange={setGap}
              color="blue"
              data={[
                { label: 'Daily', value: 'Day' },
                { label: 'Weekly', value: 'Week' },
                { label: 'Monthly', value: 'Month' },
              ]}
            />
          }
        >
          <ReportGraph data={data} height={90}></ReportGraph>
        </Section>

        <Section title="Incomes">
          <Grid>
            <Grid.Col span={4}>
              {reportData.income_rank.map((each_income) => (
                <Box mt="sm" key={each_income.account}>
                  <Group position="apart">
                    <Text>{each_income.account}</Text>
                    <Text size="xs" color="teal" weight={700}>
                      {(parseFloat(each_income.percent) * 100).toFixed(2)}%
                    </Text>
                  </Group>
                  <Progress radius="xs" size="lg" color="teal" value={parseFloat(each_income.percent) * 100} />
                </Box>
              ))}
            </Grid.Col>
            <Grid.Col span={8}>
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
                  {reportData.income_top_transactions.map((journal) => (
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

        <Section title="Expenses">
          <Grid>
            <Grid.Col span={4}>
              {reportData.expense_rank.map((each_expense) => (
                <Box mt="sm" key={each_expense.account}>
                  <Group position="apart">
                    <Text>{each_expense.account}</Text>
                    <Text size="xs" color="red" weight={700}>
                      {(parseFloat(each_expense.percent) * 100).toFixed(2)}%
                    </Text>
                  </Group>

                  <Progress radius="xs" size="lg" color="red" value={parseFloat(each_expense.percent) * 100} />
                </Box>
              ))}
            </Grid.Col>
            <Grid.Col span={8}>
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
                  {reportData.expense_top_transactions.map((journal) => (
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
