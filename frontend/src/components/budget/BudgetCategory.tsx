import { useLocalStorage } from '@mantine/hooks';
import { Buffer } from 'buffer';
import BudgetLine from './BudgetLine';
import BigNumber from 'bignumber.js';
import Amount from '../Amount';
import BackgroundProgress from '../basic/BackgroundProgress';
import { TableCell, TableRow } from '../ui/table';
import { Button } from '../ui/button';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { BudgetListItem } from '@/api/types';
interface Props {
  name: string;
  items: BudgetListItem[];
}

export default function BudgetCategory(props: Props) {
  const [isShow, setCollapse] = useLocalStorage({
    key: `budget-category-${Buffer.from(props.name).toString('base64')}-collapse`,
    defaultValue: true,
  });
  const assigned_amount = props.items.reduce(
    (accr, item) => ({
      number: accr.number.plus(new BigNumber(item.assigned_amount.number)),
      commodity: item.assigned_amount.currency,
    }),
    { number: new BigNumber(0), commodity: '' },
  );

  const activity_amount = props.items.reduce(
    (accr, item) => ({
      number: accr.number.plus(new BigNumber(item.activity_amount.number)),
      commodity: item.activity_amount.currency,
    }),
    { number: new BigNumber(0), commodity: '' },
  );

  const available_amount = props.items.reduce(
    (accr, item) => ({
      number: accr.number.plus(new BigNumber(item.available_amount.number)),
      commodity: item.available_amount.currency,
    }),
    { number: new BigNumber(0), commodity: '' },
  );
  let number = BigNumber.minimum(
    new BigNumber(activity_amount.number).div(new BigNumber(assigned_amount.number)).multipliedBy(100),
    new BigNumber(100),
  ).toFormat(2);

  return (
    <>
      <TableRow className="relative z-[1]">
        <TableCell>
          <BackgroundProgress percentage={number} />
          <div className="flex items-center gap-2">
            <Button className="hover:bg-transparent" size="icon" variant="ghost" onClick={() => setCollapse(!isShow)}>
              {isShow ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
            </Button>
            <span className="font-bold">{props.name}</span>
          </div>
        </TableCell>
        <TableCell className="text-right">
          <span className="font-bold">{number} %</span>
        </TableCell>
        <TableCell className="text-right">
          <span className="font-bold">
            <Amount amount={assigned_amount.number} currency={assigned_amount.commodity} />
          </span>
        </TableCell>
        <TableCell className="text-right">
          <span className="font-bold">
            <Amount amount={activity_amount.number} currency={activity_amount.commodity} />
          </span>
        </TableCell>
        <TableCell className="text-right">
          <span className="font-bold">
            <Amount amount={available_amount.number} currency={available_amount.commodity} />
          </span>
        </TableCell>
      </TableRow>
      {isShow && props.items.sort().map((item) => <BudgetLine key={`${item.name}`} {...item}></BudgetLine>)}
    </>
  );
}
