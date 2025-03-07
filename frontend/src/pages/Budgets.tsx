import { retrieveBudgets } from '@/api/requests.ts';
import { Button } from '@/components/ui/button.tsx';
import { Label } from '@/components/ui/label.tsx';
import { Switch } from '@/components/ui/switch.tsx';
import { Table, TableBody, TableHead, TableHeader, TableRow } from '@/components/ui/table.tsx';
import { BUDGETS_LINK } from '@/layout/Sidebar.tsx';
import { useDocumentTitle, useLocalStorage } from '@mantine/hooks';
import { format } from 'date-fns';
import { useAtomValue, useSetAtom } from 'jotai/index';
import { groupBy, sortBy } from 'lodash-es';
import { ChevronLeftIcon, ChevronRightIcon } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import BudgetCategory from '../components/budget/BudgetCategory';
import { breadcrumbAtom, titleAtom } from '../states/basic';

export default function Budgets() {
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const { t } = useTranslation();
  const [hideZeroAssignBudget, setHideZeroAssignBudget] = useLocalStorage({
    key: 'hideZeroAssignBudget',
    defaultValue: false,
  });
  const [date, setDate] = useState<Date>(new Date());
  const ledgerTitle = useAtomValue(titleAtom);
  useDocumentTitle(`Budgets - ${ledgerTitle}`);

  useEffect(() => {
    setBreadcrumb([
      BUDGETS_LINK,
      {
        label: `Budget ${format(date, 'MMM, yyyy')}`,
        uri: `/budgets?year=${date.getFullYear()}&month=${date.getMonth() + 1}`,
        noTranslate: true,
      },
    ]);
  }, [date]);

  const { loading, error, value: budgets } = useAsync(async() => {
    const res = await retrieveBudgets({ year: date.getFullYear(), month: date.getMonth() + 1 });
    return res.data.data;
  }, [date])

  if (error) return <div>failed to load</div>;
  if (loading || !budgets) return <div>loading...</div>;

  const goToMonth = (gap: number) => {
    let newDate = new Date(date);
    newDate.setMonth(newDate.getMonth() + gap);
    setDate(newDate);
  };

  return (
    <div>
      <div className="flex items-center justify-between gap-2">
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
        <Button variant="outline" color="gray">
          {t('REFRESH')}
        </Button>
      </div>
      <div className="flex items-center gap-2 mt-2">
        <Switch checked={hideZeroAssignBudget} onCheckedChange={(checked) => setHideZeroAssignBudget(checked)} />
        <Label>Hide Zero Amount Assigned Budget</Label>
      </div>

      <div className="rounded-md border mt-4">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Category</TableHead>
              <TableHead className="text-right">Percentage</TableHead>
              <TableHead className="text-right">Assigned</TableHead>
              <TableHead className="text-right">Activity</TableHead>
              <TableHead className="text-right">Available</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortBy(Object.entries(groupBy(budgets, (budget) => budget.category)), (entry) => entry[0]).map((entry) => (
              <BudgetCategory key={`${entry[0]}-${date.getFullYear()}-${date.getMonth()}`} name={entry[0]} items={entry[1]}></BudgetCategory>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
