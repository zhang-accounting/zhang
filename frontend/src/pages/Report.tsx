import { retrieveStatisticByAccountType, retrieveStatisticGraph, retrieveStatisticSummary } from '@/api/requests.ts';
import StatisticBox from '@/components/StatisticBox.tsx';
import { Button } from '@/components/ui/button.tsx';
import { Calendar } from '@/components/ui/calendar.tsx';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { REPORT_LINK } from '@/layout/Sidebar.tsx';
import { cn } from '@/lib/utils.ts';
import { useDocumentTitle } from '@mantine/hooks';
import { CalendarIcon } from '@radix-ui/react-icons';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect, useState } from 'react';
import { DateRange } from 'react-day-picker';
import { useAsync } from 'react-use';
import Amount from '../components/Amount';
import ReportGraph from '../components/ReportGraph';
import Section from '../components/Section';
import PayeeNarration from '../components/basic/PayeeNarration';
import { breadcrumbAtom, titleAtom } from '../states/basic';

export default function Report() {
  const [value, setValue] = useState<DateRange | undefined>({
    from: new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    to: new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  });

  const [dateRange, setDateRange] = useState<[Date, Date]>([
    new Date(new Date().getFullYear(), new Date().getMonth(), 1, 0, 0, 1),
    new Date(new Date().getFullYear(), new Date().getMonth() + 1, 0, 23, 59, 59),
  ]);

  const setBreadcrumb = useSetAtom(breadcrumbAtom);

  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Report - ${ledgerTitle}`);

  useEffect(() => {
    if (value?.from !== undefined && value?.to !== undefined) {
      setDateRange([value.from, value.to]);
    }
  }, [value]);

  useEffect(() => {
    setBreadcrumb([REPORT_LINK]);
  }, []);

  const {
    value: data,
    error,
    loading,
  } = useAsync(async () => {
    const res = await retrieveStatisticSummary({ from: dateRange[0]!.toISOString(), to: dateRange[1]!.toISOString() });
    return res.data.data;
  }, []);

  const { value: graph_data } = useAsync(async () => {
    const res = await retrieveStatisticGraph({ from: dateRange[0]!.toISOString(), to: dateRange[1]!.toISOString(), interval: 'Day' });
    return res.data.data;
  }, []);

  const { value: income_data } = useAsync(async () => {
    const res = await retrieveStatisticByAccountType({ account_type: 'Income', from: dateRange[0]!.toISOString(), to: dateRange[1]!.toISOString() });
    return res.data.data;
  }, []);

  const { value: expenses_data } = useAsync(async () => {
    const res = await retrieveStatisticByAccountType({ account_type: 'Expenses', from: dateRange[0]!.toISOString(), to: dateRange[1]!.toISOString() });
    return res.data.data;
  }, []);

  if (error) return <div>failed to load</div>;
  if (loading || !data) return <>loading</>;

  return (
    <>
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between gap-4 mb-4">
          <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">Report</h1>
          <Popover>
            <PopoverTrigger asChild>
              <Button id="date" variant={'outline'} className={cn('w-[300px] justify-start text-left font-normal', !value && 'text-muted-foreground')}>
                <CalendarIcon className="mr-2 h-4 w-4" />
                {value?.from ? (
                  value.to ? (
                    <>
                      {format(value.from, 'LLL dd, y')} - {format(value.to, 'LLL dd, y')}
                    </>
                  ) : (
                    format(value.from, 'LLL dd, y')
                  )
                ) : (
                  <span>Pick a date</span>
                )}
              </Button>
            </PopoverTrigger>
            <PopoverContent className="w-auto p-0" align="start">
              <Calendar initialFocus mode="range" defaultMonth={value?.from} selected={value} onSelect={setValue} numberOfMonths={2} />
            </PopoverContent>
          </Popover>
        </div>

        <div className={`grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4`}>
          <StatisticBox
            text={'ASSET_BALANCE'}
            amount={data.balance.calculated.number}
            currency={data.balance.calculated.currency}
            hint={'include assets and liabilities'}
          />
          <StatisticBox text={'INCOME'} amount={data.income.calculated.number} currency={data.income.calculated.currency} negative />
          <StatisticBox text={'EXPENSE'} amount={data.expense.calculated.number} currency={data.expense.calculated.currency} negative />
          <StatisticBox text={'TRANSACTION_COUNT'} amount={data.transaction_number.toString()} />
        </div>

        <Section title="Graph">
          <ReportGraph data={graph_data} height={20}></ReportGraph>
        </Section>

        <Card className="mt-2 rounded-sm ">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2 ">
            <CardTitle>Top 10 Incomes</CardTitle>
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
