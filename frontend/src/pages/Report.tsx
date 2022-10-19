import { useQuery } from '@apollo/client';
import { Container, Grid, Group, Progress, SegmentedControl, Title, Text, Table } from '@mantine/core';
import { DateRangePicker, DateRangePickerValue } from '@mantine/dates';
import { format } from 'date-fns';
import * as _ from 'lodash';
import { useState } from 'react';
import { Chart } from 'react-chartjs-2';
import JournalLine from '../components/JournalLine';
import Section from '../components/Section';
import StatusGroup from '../components/StatusGroup';
import { Posting, TransactionDto } from '../gql/jouralList';
import { STATISTIC, StatisticResponse } from '../gql/statistic';

const options = {
  responsive: true,
  interaction: {
    mode: 'index' as const,
    intersect: false,
  },
  stacked: false,
  scales: {
    y: {
      type: 'linear' as const,
      display: true,
      position: 'left' as const,
      beginAtZero: false,
    },
    y1: {
      type: 'linear' as const,
      display: true,
      position: 'right' as const,
      grid: {
        drawOnChartArea: false,
      },
      beginAtZero: false,
    },
  },
};

const build_chart_data = (data: StatisticResponse) => {
  console.log('Execute build chat data', data);
  const labels = data.statistic.frames.map((frame) => format(new Date(frame.end * 1000), 'MMM dd'));
  const total_dataset = data.statistic.frames.map((frame) => parseFloat(frame.total.summary.number));
  const income_dataset = data.statistic.frames.map((frame) => -1 * parseFloat(frame.income.summary.number));
  const expense_dataset = data.statistic.frames.map((frame) => parseFloat(frame.expense.summary.number));
  return {
    labels,
    datasets: [
      {
        type: 'line' as const,
        label: 'total',
        borderColor: 'rgb(255, 99, 132)',
        borderWidth: 2,
        fill: false,
        data: total_dataset,
        yAxisID: 'y',
      },
      {
        type: 'bar' as const,
        label: 'income',
        backgroundColor: 'rgb(75, 192, 192)',
        data: income_dataset,
        borderColor: 'white',
        borderWidth: 2,
        yAxisID: 'y1',
      },
      {
        type: 'bar' as const,
        label: 'expense',
        backgroundColor: 'rgb(53, 162, 235)',
        data: expense_dataset,
        yAxisID: 'y1',
      },
    ],
  };
};

function sumPostings(postings: Posting[]): number {
  return _.sumBy(postings, (posting) => parseFloat(posting?.unit?.number || posting?.inferredUnit?.number));
}
export default function Report() {
  const [value, setValue] = useState<DateRangePickerValue>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);
  const [gap, setGap] = useState(1);

  const { loading, error, data } = useQuery<StatisticResponse>(STATISTIC, {
    variables: {
      from: Math.round(new Date(value[0]!.getFullYear(), value[0]!.getMonth(), value[0]!.getDate(), 0, 0, 1).getTime() / 1000),
      to: Math.round(new Date(value[1]!.getFullYear(), value[1]!.getMonth(), value[1]!.getDate(), 23, 59, 59).getTime() / 1000),
      gap: gap,
    },
  });
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;
  // income
  const incomeData =
    data?.statistic.journals
      .flatMap((journal) => {
        switch (journal.type) {
          case 'TransactionDto':
            return journal.postings;
          default:
            return [];
        }
      })
      .filter((posting) => posting.account.accountType === 'Income') || [];

  const incomeTotal = sumPostings(incomeData);

  const incomeRank = _.sortBy(
    _.map(
      _.groupBy(incomeData, (posting) => posting.account.name),
      (postings, name) => ({ name, total: sumPostings(postings) }),
    ),
    (item) => item.total,
  );
  const incomeJournalRank =
    _.sortBy(
      data?.statistic.journals
        .filter((journal) => journal.type === 'TransactionDto')
        .filter((journal) => (journal as TransactionDto).postings.some((posting) => posting.account.accountType === 'Income')),
      (journal) => sumPostings((journal as TransactionDto).postings.filter((posting) => posting.account.accountType === 'Income')),
    ) || [];

  const expenseData =
    data?.statistic.journals
      .flatMap((journal) => {
        switch (journal.type) {
          case 'TransactionDto':
            return journal.postings;
          default:
            return [];
        }
      })
      .filter((posting) => posting.account.accountType === 'Expenses') || [];

  const expenseTotal = sumPostings(expenseData);

  const expenseRank = _.sortBy(
    _.map(
      _.groupBy(expenseData, (posting) => posting.account.name),
      (postings, name) => ({ name, total: sumPostings(postings) }),
    ),
    (item) => item.total,
  );

  const expenseJournalRank =
    _.sortBy(
      data?.statistic.journals
        .filter((journal) => journal.type === 'TransactionDto')
        .filter((journal) => (journal as TransactionDto).postings.some((posting) => posting.account.accountType === 'Expenses')),
      (journal) => sumPostings((journal as TransactionDto).postings.filter((posting) => posting.account.accountType === 'Expenses')),
    ).reverse() || [];

  return (
    <>
      <Container fluid>
        <Title order={2}>Report</Title>

        <Group position="right">
          <DateRangePicker placeholder="Pick dates range" value={value} onChange={setValue} />
        </Group>

        <StatusGroup
          data={[
            { title: '资产余额', amount: data?.statistic.total.summary.number, currency: data?.statistic.total.summary.currency },
            { title: '总收支', amount: data?.statistic.incomeExpense.summary.number, currency: data?.statistic.incomeExpense.summary.currency },
            { title: '收入', amount: data?.statistic.income.summary.number, currency: data?.statistic.income.summary.currency },
            { title: '支出', amount: data?.statistic.expense.summary.number, currency: data?.statistic.expense.summary.currency },
            { title: '交易数', number: data?.statistic.journals.length },
          ]}
        />

        <Section
          title="Graph"
          rightSection={
            <SegmentedControl
              size="xs"
              value={gap.toString()}
              onChange={(e) => setGap(parseInt(e))}
              color="blue"
              data={[
                { label: 'Daily', value: '1' },
                { label: 'Weekly', value: '7' },
                { label: 'Monthly', value: '30' },
              ]}
            />
          }>
          <Chart type="bar" data={build_chart_data(data!)} options={options} height={100} />
        </Section>

        <Section title="Incomes">
          <Grid>
            <Grid.Col span={4}>
              {_.take(incomeRank, 10).map((each_income) => (
                <div key={each_income.name}>
                  <Text>{each_income.name}</Text>
                  <Progress
                    sections={[
                      {
                        value: Math.round((each_income.total / incomeTotal) * 100),
                        color: 'pink',
                        label: `${Math.round((each_income.total / incomeTotal) * 10000) / 100}%`,
                        tooltip: `${each_income.total}`,
                      },
                    ]}
                    size="md"
                  />
                </div>
              ))}
            </Grid.Col>
            <Grid.Col span={8}>
              <Table verticalSpacing="xs" highlightOnHover>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th style={{}}>Payee & Narration</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {_.take(incomeJournalRank, 10).map((journal, idx) => (
                    <JournalLine key={idx} data={journal} />
                  ))}
                </tbody>
              </Table>
            </Grid.Col>
          </Grid>
        </Section>

        <Section title="Expenses">
          <Grid>
            <Grid.Col span={4}>
              {_.take(expenseRank, 10).map((each_income) => (
                <div key={each_income.name}>
                  <Text>{each_income.name}</Text>
                  <Progress
                    sections={[
                      {
                        value: Math.round((each_income.total / expenseTotal) * 100),
                        color: 'pink',
                        label: `${Math.round((each_income.total / expenseTotal) * 10000) / 100}%`,
                        tooltip: Math.round((each_income.total / expenseTotal) * 10000) / 100,
                      },
                    ]}
                    size="md"
                  />
                </div>
              ))}
            </Grid.Col>
            <Grid.Col span={8}>
              <Table verticalSpacing="xs" highlightOnHover>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th style={{}}>Payee & Narration</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {_.take(expenseJournalRank, 10).map((journal, idx) => (
                    <JournalLine key={idx} data={journal} />
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
