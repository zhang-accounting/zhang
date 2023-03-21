import { JournalItem } from '../../../rest-model';
import TableViewBalancePadLine from './TableViewBalancePadLine';
import TableViewTransactionLine from './TableViewTransactionLine';
import TableViewBalanceCheckLine from './TableViewBalanceCheckLine';

interface Props {
  data: JournalItem;
}
export default function TableViewJournalLine({ data }: Props) {
  let line = null;
  switch (data.type) {
    case 'BalanceCheck':
      line = <TableViewBalanceCheckLine data={data} />;
      break;
    case 'BalancePad':
      line = <TableViewBalancePadLine data={data} />;
      break;
    case 'Transaction':
      line = <TableViewTransactionLine data={data} />;
      break;
  }
  return line;
}
