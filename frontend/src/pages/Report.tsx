import { useEffect, useState } from 'react';
import useSWR from 'swr';
import Amount from '../components/Amount';
import ReportGraph from '../components/ReportGraph';
import Section from '../components/Section';
import { StatisticGraphResponse, StatisticResponse, StatisticTypeResponse } from '../rest-model';
import PayeeNarration from '../components/basic/PayeeNarration';
import { useDocumentTitle } from '@mantine/hooks';
import { useAtomValue } from 'jotai/index';
import { titleAtom } from '../states/basic';
import { fetcher } from '../global.ts';
import { Table, TableBody, TableHead } from '@/components/ui/table.tsx';
import { TableRow } from '@/components/ui/table.tsx';
import { TableHeader } from '@/components/ui/table.tsx';
import { TableCell } from '@/components/ui/table.tsx';
import { Button } from '@/components/ui/button.tsx';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover.tsx';
import { cn } from '@/lib/utils.ts';
import { CalendarIcon } from '@radix-ui/react-icons';
import { format } from 'date-fns';
import { Calendar } from '@/components/ui/calendar.tsx';
import { DateRange } from 'react-day-picker';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import StatisticBox from '@/components/StatisticBox.tsx';


const color_set = ['hsl(var(--chart-1))', 'hsl(var(--chart-2))', 'hsl(var(--chart-3))', 'hsl(var(--chart-4))', 'hsl(var(--chart-5))'];



export default function Report() {
  const [value, setValue] = useState<DateRange | undefined>({
    from: new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    to: new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  });

  const [dateRange, setDateRange] = useState<[Date, Date]>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Report - ${ledgerTitle}`);

  useEffect(() => {
    if (value?.from !== undefined && value?.to !== undefined) {
      setDateRange([value.from, value.to]);
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


  if (!graph_data) return <>loading</>;

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <>
      <div className='flex flex-col gap-4'>

        <div className='flex items-center justify-between gap-4 mb-4'>
          <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Report</h1>
          <Popover>
            <PopoverTrigger asChild>
              <Button
                id="date"
                variant={"outline"}
                className={cn(
                  "w-[300px] justify-start text-left font-normal",
                  !value && "text-muted-foreground"
                )}
              >
                <CalendarIcon className="mr-2 h-4 w-4" />
                {value?.from ? (
                  value.to ? (
                    <>
                      {format(value.from, "LLL dd, y")} -{" "}
                      {format(value.to, "LLL dd, y")}
                    </>
                  ) : (
                    format(value.from, "LLL dd, y")
                  )
                ) : (
                  <span>Pick a date</span>
                )}
              </Button>
            </PopoverTrigger>
            <PopoverContent className="w-auto p-0" align="start">
              <Calendar
                initialFocus
                mode="range"
                defaultMonth={value?.from}
                selected={value}
                onSelect={setValue}
                numberOfMonths={2}
              />
            </PopoverContent>
          </Popover>
        </div>

        <div className={`grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4`}>
          <StatisticBox
            text={'资产余额'}
            amount={data.balance.calculated.number}
            currency={data.balance.calculated.currency}
            hint={'include assets and liabilities'}
          />
          <StatisticBox text={'收入'} amount={data.income.calculated.number}
            currency={data.income.calculated.currency} negative />
          <StatisticBox text={'支出'} amount={data.expense.calculated.number}
            currency={data.expense.calculated.currency} negative />
          <StatisticBox text={'交易数'} amount={data.transaction_number.toString()}
          />
        </div>

        <Section title="Graph">
          <ReportGraph data={graph_data} height={20}></ReportGraph>
        </Section>

        <Card className="mt-2 rounded-sm ">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
            <CardTitle>Top 10 Incomes</CardTitle>
          </CardHeader>
          <CardContent>
            <Table >
              <TableHeader>
                <TableRow>
                  <TableHead>Date</TableHead>
                  <TableHead>Account</TableHead>
                  <TableHead style={{}}>Payee & Narration</TableHead>
                  <TableHead>Amount</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
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
              </TableBody>
            </Table>
          </CardContent>
        </Card>



        <Card className="mt-2 rounded-sm ">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
            <CardTitle>Top 10 Expenses</CardTitle>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Date</TableHead>
                  <TableHead>Account</TableHead>
                  <TableHead style={{}}>Payee & Narration</TableHead>
                  <TableHead>Amount</TableHead>
                </TableRow>
              </TableHeader>
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
          </CardContent>
        </Card>
      </div>
    </>
  );
}
