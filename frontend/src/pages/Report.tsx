import {useQuery} from '@apollo/client';
import {Container, Grid, Group, Progress, SegmentedControl, Table, Text, Title} from '@mantine/core';
import {DateRangePicker, DateRangePickerValue} from '@mantine/dates';
import {format} from 'date-fns';
import {} from 'lodash';
import {groupBy, map, sortBy, sumBy, take} from 'lodash-es';
import {useEffect, useState} from 'react';
import {Chart} from 'react-chartjs-2';
import Section from '../components/Section';
import StatusGroup from '../components/StatusGroup';
import {Posting, TransactionDto} from '../gql/jouralList';
import useSWR from "swr";
import {fetcher} from "../index";
import BigNumber from "bignumber.js";
import {AccountType, StatisticResponse} from "../rest-model";

const options = (isLogarithmic: boolean, offset: number) => ({
    responsive: true,
    interaction: {
        mode: 'index' as const,
        intersect: false,
    },
    stacked: false,
    plugins: {
        tooltip: {
            position: 'nearest' as const,
            callbacks: {

                title: () => {
                    return "tooltip callback"
                },
                label: (item: any) => {

                    if (item.dataset.label === 'total') {
                        const valueWithOffset = parseFloat(item.formattedValue) + offset;
                        return `${item.dataset.label}: ${valueWithOffset}`
                    }
                    return `${item.dataset.label}: ${item.formattedValue}`
                }
            }
        }
    },
    scales: {
        total: {
            type: isLogarithmic ? 'logarithmic' as const : 'linear' as const,
            display: false,
            position: 'left' as const,
            beginAtZero: false,
            ticks: {
                callback: function (value: any, _index: any, _ticks: any) {
                    return parseFloat(value) + offset;
                }
            }
        },
        bar: {
            type: 'linear' as const,
            display: false,
            position: 'right' as const,
            grid: {
                drawOnChartArea: false,
            },
        },
    },
});
const build_chart_data = (data: StatisticResponse) => {
    const dates = sortBy(Object.keys(data.changes).map(date => [date, new Date(date)]), item => item[1]);

    const sequencedDate = dates.map(date => date[0] as string);

    const labels = dates.map((date) => format(date[1] as Date, 'MMM dd'));

    let total_dataset = sequencedDate.map(date => {
        const target_day = data.details[date] ?? {};
        let total = new BigNumber(0);
        Object.entries(target_day).filter(it =>
            it[0].startsWith(AccountType.Assets) || it[0].startsWith(AccountType.Liabilities)
        ).forEach(it => {
            total = total.plus(new BigNumber(it[1].number))
        })
        return total.toNumber();
    });

    // let total_dataset = data.statistic.frames.map((frame) => parseFloat(frame.total.summary.number));
    const isLogarithmic = total_dataset.every(item => item >= 0);
    const min = Math.min.apply(null, total_dataset) - 50;
    if (isLogarithmic) {
        total_dataset = total_dataset.map(item => item - min);
    }
    const income_dataset = sequencedDate.map(date => -1 * parseFloat(data.changes[date]?.[AccountType.Income]?.number ?? 0))
    const expense_dataset = sequencedDate.map(date => parseFloat(data.changes[date]?.[AccountType.Expenses]?.number ?? 0))
    console.log("incom", income_dataset, expense_dataset);
    console.log('income_dataset', income_dataset, expense_dataset);
    return {
        data: {
            labels,
            datasets: [
                {
                    type: 'line' as const,
                    label: 'total',
                    borderColor: 'rgb(255, 99, 132)',
                    borderWidth: 2,
                    data: total_dataset,
                    yAxisID: 'total',
                },
                {
                    type: 'bar' as const,
                    label: 'income',
                    backgroundColor: 'rgb(17, 183, 205)',
                    data: income_dataset,
                    borderColor: 'white',
                    borderRadius: 3,
                    yAxisID: 'bar',
                },
                {
                    type: 'bar' as const,
                    label: 'expense',
                    backgroundColor: 'rgb(247, 31, 167)',
                    borderRadius: 3,
                    data: expense_dataset,
                    yAxisID: 'bar',
                },
            ],
        }, isLogarithmic, offset: isLogarithmic ? min : 0
    };
};

function sumPostings(postings: Posting[]): number {
    return sumBy(postings, (posting) => parseFloat(posting?.unit?.number || posting?.inferredUnit?.number));
}

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
            console.log("update value", value);
            setDateRange([value[0], value[1]]);
        }
    }, [value])

    const [gap, setGap] = useState("Day");

    const {
        data,
        error
    } = useSWR<StatisticResponse>(`/api/statistic?from=${dateRange[0]!.toISOString()}&to=${dateRange[1]!.toISOString()}&interval=${gap}`, fetcher)

    if (error) return <div>failed to load</div>;
    if (!data) return <>loading</>;

    const chart_info = build_chart_data(data);
    return (
        <>
            <Container fluid>
                <Group position="apart" my="lg">
                    <Title order={2}>Report</Title>
                    <DateRangePicker placeholder="Pick dates range" value={value} onChange={setValue}/>
                </Group>

                {/*<StatusGroup*/}
                {/*  data={[*/}
                {/*    { title: '资产余额', amount: data?.statistic.total.summary.number, currency: data?.statistic.total.summary.currency },*/}
                {/*    { title: '总收支', amount: data?.statistic.incomeExpense.summary.number, currency: data?.statistic.incomeExpense.summary.currency },*/}
                {/*    { title: '收入', amount: data?.statistic.income.summary.number, currency: data?.statistic.income.summary.currency },*/}
                {/*    { title: '支出', amount: data?.statistic.expense.summary.number, currency: data?.statistic.expense.summary.currency },*/}
                {/*    { title: '交易数', number: data?.statistic.journals.length },*/}
                {/*  ]}*/}
                {/*/>*/}

                <Section
                    title="Graph"
                    rightSection={
                        <SegmentedControl
                            size="xs"
                            value={gap.toString()}
                            onChange={setGap}
                            color="blue"
                            data={[
                                {label: 'Daily', value: 'Day'},
                                {label: 'Weekly', value: 'Week'},
                                {label: 'Monthly', value: 'Month'},
                            ]}
                        />
                    }>
                    <Chart type="line" data={chart_info.data}
                           options={options(chart_info.isLogarithmic, chart_info.offset)}/>

                </Section>

                {/*<Section title="Incomes">*/}
                {/*  <Grid>*/}
                {/*    <Grid.Col span={4}>*/}
                {/*      {take(incomeRank, 10).map((each_income) => (*/}
                {/*        <div key={each_income.name}>*/}
                {/*          <Text>{each_income.name}</Text>*/}
                {/*          <Progress*/}
                {/*            sections={[*/}
                {/*              {*/}
                {/*                value: Math.round((each_income.total / incomeTotal) * 100),*/}
                {/*                color: 'pink',*/}
                {/*                label: `${Math.round((each_income.total / incomeTotal) * 10000) / 100}%`,*/}
                {/*                tooltip: `${each_income.total}`,*/}
                {/*              },*/}
                {/*            ]}*/}
                {/*            size="md"*/}
                {/*          />*/}
                {/*        </div>*/}
                {/*      ))}*/}
                {/*    </Grid.Col>*/}
                {/*    <Grid.Col span={8}>*/}
                {/*      <Table verticalSpacing="xs" highlightOnHover>*/}
                {/*        <thead>*/}
                {/*          <tr>*/}
                {/*            <th>Date</th>*/}
                {/*            <th style={{}}>Payee & Narration</th>*/}
                {/*            <th></th>*/}
                {/*          </tr>*/}
                {/*        </thead>*/}
                {/*        <tbody>*/}
                {/*          {take(incomeJournalRank, 10).map((journal, idx) => (*/}
                {/*            // <JournalLine key={idx} data={journal} />*/}
                {/*            <div>line</div>*/}
                {/*          ))}*/}
                {/*        </tbody>*/}
                {/*      </Table>*/}
                {/*    </Grid.Col>*/}
                {/*  </Grid>*/}
                {/*</Section>*/}

                {/*<Section title="Expenses">*/}
                {/*  <Grid>*/}
                {/*    <Grid.Col span={4}>*/}
                {/*      {take(expenseRank, 10).map((each_income) => (*/}
                {/*        <div key={each_income.name}>*/}
                {/*          <Text>{each_income.name}</Text>*/}
                {/*          <Progress*/}
                {/*            sections={[*/}
                {/*              {*/}
                {/*                value: Math.round((each_income.total / expenseTotal) * 100),*/}
                {/*                color: 'pink',*/}
                {/*                label: `${Math.round((each_income.total / expenseTotal) * 10000) / 100}%`,*/}
                {/*                tooltip: Math.round((each_income.total / expenseTotal) * 10000) / 100,*/}
                {/*              },*/}
                {/*            ]}*/}
                {/*            size="md"*/}
                {/*          />*/}
                {/*        </div>*/}
                {/*      ))}*/}
                {/*    </Grid.Col>*/}
                {/*    <Grid.Col span={8}>*/}
                {/*      <Table verticalSpacing="xs" highlightOnHover>*/}
                {/*        <thead>*/}
                {/*          <tr>*/}
                {/*            <th>Date</th>*/}
                {/*            <th style={{}}>Payee & Narration</th>*/}
                {/*            <th></th>*/}
                {/*          </tr>*/}
                {/*        </thead>*/}
                {/*        <tbody>*/}
                {/*          {take(expenseJournalRank, 10).map((journal, idx) => (*/}
                {/*            // <JournalLine key={idx} data={journal} />*/}
                {/*            <div> line</div>*/}
                {/*          ))}*/}
                {/*        </tbody>*/}
                {/*      </Table>*/}
                {/*    </Grid.Col>*/}
                {/*  </Grid>*/}
                {/*</Section>*/}
            </Container>
        </>
    );
}
