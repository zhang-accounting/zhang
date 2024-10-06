import { BudgetListItem } from '../../rest-model';
import Amount from '../Amount';
import BigNumber from 'bignumber.js';
import BackgroundProgress from '../basic/BackgroundProgress';
import { useNavigate } from 'react-router';
import { TableCell } from '../ui/table';
import { TableRow } from '../ui/table';

interface Props extends BudgetListItem {}

export default function BudgetLine(props: Props) {
  const navigate = useNavigate();
  let number = BigNumber.minimum(new BigNumber(props.activity_amount.number).div(new BigNumber(props.assigned_amount.number)).multipliedBy(100), 100).toFormat(
    2,
  );
  return (
    <TableRow className="relative z-[1]">
      <TableCell className="py-3">
        <BackgroundProgress percentage={number} />
        <div className="flex items-center gap-2">
          <div className="w-9 h-4 bg-transparent"></div>
          <span className="cursor-pointer" onClick={() => navigate(props.name)}>
            {props.alias ?? props.name}
          </span>
        </div>
      </TableCell>
      <TableCell style={{ textAlign: 'end' }}>{number} %</TableCell>
      <TableCell style={{ textAlign: 'end' }}>
        <Amount amount={props.assigned_amount.number} currency={props.assigned_amount.currency} />
      </TableCell>
      <TableCell style={{ textAlign: 'end' }}>
        <Amount amount={props.activity_amount.number} currency={props.activity_amount.currency} />
      </TableCell>
      <TableCell style={{ textAlign: 'end' }}>
        <Amount amount={props.available_amount.number} currency={props.available_amount.currency} />
      </TableCell>
    </TableRow>
  );
}
