import { JournalItem } from '../../../rest-model';
import TableViewBalancePadLine from './TableViewBalancePadLine';
import TableViewTransactionLine from './TableViewTransactionLine';
interface Props {
  data: JournalItem;
}
export default function TableViewJournalLine({ data }: Props) {
  let line = null;
  switch (data.type) {
    // case 'BalanceCheck':
    //   line = <BalanceCheckLine data={data} onClick={onClick} />;
    //   break;
    case 'BalancePad':
      line = <TableViewBalancePadLine data={data} />;
      break;
    case 'Transaction':
      line = <TableViewTransactionLine data={data} />;
      break;
  }
  return line;
}
