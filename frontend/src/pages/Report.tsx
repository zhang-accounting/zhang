import { useQuery } from "@apollo/client";
import { Box, Flex, Heading, Progress, ProgressLabel, Select, Stat, StatLabel, StatNumber } from "@chakra-ui/react";
// @ts-ignore
import DateRangePicker from '@wojtekmaj/react-daterange-picker';
import { format } from "date-fns";
import { setValues } from "framer-motion/types/render/utils/setters";
import * as _ from 'lodash';
import { useState } from "react";
import { Chart } from 'react-chartjs-2';
import Amount from "../components/Amount";
import Block from "../components/Block";
import JournalLine from "../components/JournalLine";
import { Posting, TransactionDto } from "../gql/jouralList";
import { STATISTIC, StatisticResponse } from "../gql/statistic";


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
            beginAtZero: false
        },
        y1: {
            type: 'linear' as const,
            display: true,
            position: 'right' as const,
            grid: {
                drawOnChartArea: false,
            },
            beginAtZero: false
        },
    },
};

const build_chart_data = (data: StatisticResponse) => {
    console.log("Execute build chat data", data);
    const labels = data.statistic.frames.map(frame => format(new Date(frame.end * 1000), 'MMM dd'));
    const total_dataset = data.statistic.frames.map(frame => parseFloat(frame.total.summary.number));
    const income_dataset = data.statistic.frames.map(frame => -1 * parseFloat(frame.income.summary.number));
    const expense_dataset = data.statistic.frames.map(frame => parseFloat(frame.expense.summary.number));
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
    }
}

function sumPostings(postings: Posting[]): number {
    return _.sumBy(postings, posting => parseFloat(posting.unit.number));
}
export default function Report() {
    const now = new Date();
    const begining_time = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 1);
    const end_time = new Date(now.getFullYear(), now.getMonth() + 1, 0, 23, 59, 59);
    const [value, setValue] = useState([begining_time, end_time]);
    const [gap, setGap] = useState(1);

    const { loading, error, data } = useQuery<StatisticResponse>(STATISTIC, {
        variables: {
            from: Math.round(new Date(value[0].getFullYear(), value[0].getMonth(), value[0].getDate(), 0, 0, 1).getTime() / 1000),
            to: Math.round(new Date(value[1].getFullYear(), value[1].getMonth(), value[1].getDate(), 23, 59, 59).getTime() / 1000),
            gap: gap
        }
    });
    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;
    // income
    const incomeData = data?.statistic.journals.flatMap(journal => {
        switch (journal.type) {
            case "TransactionDto":
                return journal.postings
            default:
                return [];
        }
    }).filter(posting => posting.account.accountType === "Income") || [];

    const incomeTotal = sumPostings(incomeData);

    const incomeRank = _.sortBy(_.map(_.groupBy(incomeData, posting => posting.account.name), (postings, name) => ({ name, total: sumPostings(postings) })), item => item.total);
    const incomeJournalRank = _.sortBy(
        data?.statistic.journals
            .filter(journal => journal.type === "TransactionDto")
            .filter(journal => (journal as TransactionDto).postings.some(posting => posting.account.accountType === "Income")),
        journal => sumPostings((journal as TransactionDto).postings.filter(posting => posting.account.accountType === "Income"))
    )
        || [];


    const expenseData = data?.statistic.journals.flatMap(journal => {
        switch (journal.type) {
            case "TransactionDto":
                return journal.postings
            default:
                return [];
        }
    }).filter(posting => posting.account.accountType === "Expenses") || [];

    const expenseTotal = sumPostings(expenseData);

    const expenseRank = _.sortBy(_.map(_.groupBy(expenseData, posting => posting.account.name), (postings, name) => ({ name, total: sumPostings(postings) })), item => item.total);

    const expenseJournalRank = _.sortBy(
        data?.statistic.journals
            .filter(journal => journal.type === "TransactionDto")
            .filter(journal => (journal as TransactionDto).postings.some(posting => posting.account.accountType === "Expenses")),
        journal => sumPostings((journal as TransactionDto).postings.filter(posting => posting.account.accountType === "Expenses")),
    ).reverse()
        || [];

    return (

        <div>

            <Flex>
                <Box><Heading>Report</Heading></Box>
                <Box>
                    <DateRangePicker onChange={setValue} value={value} />
                </Box>

                <Box><Select size="sm" onChange={e => setGap(parseInt(e.target.value))} value={gap}>
                    <option value={"1"}>Daily</option>
                    <option value={"7"}>Weekly</option>
                    <option value={"30"}>Monthly</option>
                </Select></Box>
            </Flex>
            <Flex>
                <Stat>
                    <StatLabel>资产余额</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.total.summary.number} currency={data?.statistic.total.summary.currency} /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>总收支</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.incomeExpense.summary.number} currency={data?.statistic.incomeExpense.summary.currency} negetive /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>收入</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.income.summary.number} currency={data?.statistic.income.summary.currency} negetive /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>支出</StatLabel>
                    <StatNumber><Amount amount={data?.statistic.expense.summary.number} currency={data?.statistic.expense.summary.currency} /></StatNumber>
                </Stat>
                <Stat>
                    <StatLabel>交易数</StatLabel>
                    <StatNumber>{data?.statistic.journals.length}</StatNumber>
                </Stat>
            </Flex>
            <Box m={1}>
                <Block title="Graph">
                    <Chart type='bar' data={build_chart_data(data!)} options={options} height={100} />
                </Block>
            </Box>

            <Flex>
                <Box flex="0 0 30%" m={1}>
                    <Block title="收入占比">
                        {_.take(incomeRank, 10).map(each_income =>
                            <Box key={each_income.name} pb={1}>
                                <p>{each_income.name}</p>
                                <Progress value={each_income.total} max={incomeTotal} size='md' >
                                    <ProgressLabel textAlign={"left"}>{Math.round(each_income.total / incomeTotal * 10000) / 100}%</ProgressLabel>
                                </Progress>
                            </Box>
                        )}
                    </Block>
                </Box>
                <Box flex="1" m={1}>
                    <Block title="收入排行">
                        <div>
                            {_.take(incomeJournalRank, 10).map((journal, idx) => <JournalLine key={idx} data={journal} />)}
                        </div>
                    </Block>
                </Box>
            </Flex>
            <Flex>
                <Box flex="0 0 30%" m={1}>
                    <Block title="支出占比">
                        {_.take(expenseRank, 10).map(each_income =>
                            <Box key={each_income.name} pb={1}>
                                <p>{each_income.name}</p>
                                <Progress value={each_income.total} max={expenseTotal} size='md' >
                                    <ProgressLabel textAlign={"left"}>{Math.round(each_income.total / expenseTotal * 10000) / 100}%</ProgressLabel>
                                </Progress>
                            </Box>
                        )}
                    </Block>
                </Box>
                <Box flex="1" m={1}>
                    <Block title="支出排行">
                        <div>
                            {_.take(expenseJournalRank, 10).map((journal, idx) => <JournalLine key={idx} data={journal} />)}
                        </div>
                    </Block>
                </Box>
            </Flex>




        </div>
    )
}


