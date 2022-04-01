import { gql, useQuery } from "@apollo/client";
import { Flex } from "@chakra-ui/react";
import StatisticBox from "./StatisticBox";

export default function Component({ }) {
  const now = new Date();
  const begining_time = new Date(now.getFullYear(), now.getMonth(), 1, 0, 0, 1);
  const end_time = new Date(now.getFullYear(), now.getMonth() + 1, 0, 23, 59, 59);

  const { loading, error, data } = useQuery(gql`
  query BAR_STATISTIC($from: Int, $to: Int) {
    statistic(from: $from, to: $to) {
      start
      end
      total: categorySnapshot(categories: ["Assets", "Liabilities"]) {
        summary {
          number
          currency
        }
        detail {
          number
          currency
        }
      }
      monthIncome: distance(accounts: ["Income"]) {
        summary {
          number
          currency
        }
        detail {
          number
          currency
        }
      }
      monthExpense: distance(accounts: ["Expenses"]) {
        summary {
          number
          currency
        }
        detail {
          number
          currency
        }
      }
  
      liability: categorySnapshot(categories: ["Liabilities"]) {
        summary {
          number
          currency
        }
        detail {
          number
          currency
        }
      }
    }
  }
    `, {
    variables: {
      from: Math.round(begining_time.getTime() / 1000),
      to: Math.round(end_time.getTime() / 1000)
    }
  })
  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;
  return (
    <Flex >
      <StatisticBox text={"资产余额"} amount={data.statistic.total.summary.number} currency={data.statistic.total.summary.currency} />
      <StatisticBox text={"负债"} amount={data.statistic.liability.summary.number} currency={data.statistic.liability.summary.currency} negetive />
      <StatisticBox text={"本月收入"} amount={data.statistic.monthIncome.summary.number} currency={data.statistic.monthIncome.summary.currency} negetive />
      <StatisticBox text={"本月支出"} amount={data.statistic.monthExpense.summary.number} currency={data.statistic.monthExpense.summary.currency} />
    </Flex>
  )
}