import { Dispatch, SetStateAction } from 'react';
import { JournalItem } from '../gql/jouralList';
import BalanceCheckLine from './BalanceCheckLine';
import BalancePadLine from './BalancePadLine';
import TransactionLine from './TransactionLine';
interface Props {
  data: JournalItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}
export default function JournalLine({ data, onClick }: Props) {
  let line = null;
  switch (data.type) {
    case 'BalanceCheckDto':
      line = <BalanceCheckLine data={data} onClick={onClick} />;
      break;
    case 'BalancePadDto':
      line = <BalancePadLine data={data} onClick={onClick} />;
      break;
    case 'TransactionDto':
      line = <TransactionLine data={data} onClick={onClick} />;
      break;
  }
  return line;
}
