import { JournalItem } from '@/api/types';
import BalanceCheckPreview from './BalanceCheckPreview';
import BalancePadPreview from './BalancePadPreview';
import TransactionPreview from './TransactionPreview';

interface Props {
  data?: JournalItem;
}

export default function JournalPreview(props: Props) {
  let line = null;
  if (!props.data) {
    return <div>preview click</div>;
  }
  switch (props.data.type) {
    case 'BalanceCheck':
      line = <BalanceCheckPreview data={props.data} />;
      break;
    case 'BalancePad':
      line = <BalancePadPreview data={props.data} />;
      break;
    case 'Transaction':
      line = <TransactionPreview data={props.data} />;
      break;
  }
  return line;
}
