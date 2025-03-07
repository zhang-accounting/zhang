import { retrieveBudgetEvent, retrieveBudgetInfo } from '@/api/requests.ts';
import { Badge } from '@/components/ui/badge.tsx';
import { Button } from '@/components/ui/button.tsx';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card.tsx';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { BUDGETS_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle } from '@mantine/hooks';
import { ChevronLeftIcon, ChevronRightIcon } from '@radix-ui/react-icons';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { useEffect, useState } from 'react';
import { useParams } from 'react-router';
import { useAsync } from 'react-use';
import Amount from '../components/Amount';
import PayeeNarration from '../components/basic/PayeeNarration';
import { breadcrumbAtom, titleAtom } from '../states/basic';

function SingleBudget() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  let { budgetName } = useParams();
  const [date, setDate] = useState<Date>(new Date());
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`${budgetName} | Budgets - ${ledgerTitle}`);
  useEffect(() => {
    setBreadcrumb([
      BUDGETS_LINK,
      {
        label: budgetName ?? '',
        uri: `/budgets/${budgetName}`,
        noTranslate: true,
      },
    ]);
  }, [budgetName]);

  const goToMonth = (gap: number) => {
    let newDate = new Date(date);
    newDate.setMonth(newDate.getMonth() + gap);
    setDate(newDate);
  };
  const { value: budget_info, error } = useAsync(async () => {
    const res = await retrieveBudgetInfo({ budget_name: budgetName ?? '' });
    return res.data.data;
  }, [budgetName]);
  const { value: budget_interval_event } = useAsync(async () => {
    const res = await retrieveBudgetEvent({ budget_name: budgetName ?? '', year: date.getFullYear(), month: date.getMonth() + 1 });
    return res.data.data;
  }, [budgetName, date]);

  if (error) return <div>failed to load</div>;
  if (!budget_info) return <div>{error}</div>;
  return (
    <div>
      <div className="grid grid-cols-12 gap-4">
        <Card className="mt-2 rounded-sm  col-span-8">
          <CardHeader className="flex flex-row  justify-between space-y-0 pb-2 bg-gray-100">
            <CardTitle>
              <div className="flex items-center gap-2">
                <h1 className="flex-1 shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">{budget_info.alias ?? budget_info.name}</h1>
                {budget_info.alias && (
                  <Badge variant="outline" className="text-sm text-gray-500">
                    {budget_info.name}
                  </Badge>
                )}
              </div>
            </CardTitle>
            <div className="flex items-center gap-2">
              <Button variant="ghost" onClick={() => goToMonth(-1)}>
                <ChevronLeftIcon className="h-4 w-4" />
              </Button>
              <h1 className="inline-block shrink-0 whitespace-nowrap text-xl font-semibold tracking-tight sm:grow-0">{`${format(date, 'MMM, yyyy')}`}</h1>
              <Button
                variant="ghost"
                onClick={() => goToMonth(1)}
                disabled={date.getFullYear() === new Date().getFullYear() && date.getMonth() === new Date().getMonth()}
              >
                <ChevronRightIcon className="h-4 w-4" />
              </Button>
            </div>
          </CardHeader>
          <CardContent className=" mt-4 text-sm">
            <div className="grid gap-3">
              <div className="font-semibold">Related Accounts</div>

              <div className="flex flex-wrap gap-2">
                {budget_info.related_accounts.map((account) => (
                  <Badge key={account}>{account}</Badge>
                ))}
              </div>
            </div>
          </CardContent>
        </Card>

        <Card className="mt-2 rounded-sm col-span-4">
          <CardHeader className="flex flex-row  justify-between space-y-0 pb-2 bg-gray-100">
            <CardTitle>Budget Balance</CardTitle>
          </CardHeader>
          <CardContent className=" mt-4 text-sm">
            <ul className="grid gap-3">
              <li className="flex items-center justify-between">
                <span className="text-muted-foreground">Assigned Amount</span>
                <Amount amount={budget_info.assigned_amount.number} currency={budget_info.assigned_amount.currency}></Amount>
              </li>

              <li className="flex items-center justify-between">
                <span className="text-muted-foreground">Activity Amount</span>
                <Amount amount={budget_info.activity_amount.number} currency={budget_info.activity_amount.currency}></Amount>
              </li>

              <li className="flex items-center justify-between">
                <span className="text-muted-foreground">Available Amount</span>
                <Amount amount={budget_info.available_amount.number} currency={budget_info.available_amount.currency}></Amount>
              </li>
            </ul>
          </CardContent>
        </Card>
      </div>

      <div className="rounded-md border mt-4">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Date</TableHead>
              <TableHead>Activity</TableHead>
              <TableHead>Account</TableHead>
              <TableHead style={{ textAlign: 'end' }}>Assigned Amount</TableHead>
              <TableHead style={{ textAlign: 'end' }}>Activity Amount</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {(budget_interval_event ?? []).map((it) => {
              return (
                <TableRow>
                  <TableCell>{format(it.timestamp * 1000, 'MMM dd HH:mm:ss')}</TableCell>
                  <TableCell>{'event_type' in it ? it.event_type : <PayeeNarration payee={it.payee} narration={it.narration} />}</TableCell>
                  <TableCell>{!('event_type' in it) && <Badge>{it.account}</Badge>}</TableCell>
                  <TableCell style={{ textAlign: 'end' }}>{'event_type' in it && <Amount amount={it.amount?.number!} currency={it.amount?.currency!} />}</TableCell>
                  <TableCell style={{ textAlign: 'end' }}>
                    {!('event_type' in it) && <Amount amount={it.inferred_unit_number} currency={it.inferred_unit_commodity} />}
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

export default SingleBudget;
