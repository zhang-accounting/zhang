import { Dispatch, SetStateAction } from 'react';
import { JournalItem } from '../rest-model';
import TransactionLine from './TransactionLine';
interface Props {
  data: JournalItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}
export default function JournalLine({ data, onClick }: Props) {
  let line = null;
  switch (data.type) {
    // case 'BalanceCheck':
    //   line = <BalanceCheckLine data={data} onClick={onClick} />;
    //   break;
    // case 'BalancePad':
    //   line = <BalancePadLine data={data} onClick={onClick} />;
    //   break;
    case 'Transaction':
      line = <TransactionLine data={data} onClick={onClick} />;
      break;
  }
  return line;
}
