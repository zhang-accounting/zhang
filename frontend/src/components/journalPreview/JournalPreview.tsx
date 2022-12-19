import { JournalItem, JournalTransactionItem } from '../../rest-model';
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
    // case 'BalanceCheck':
    //   line = <div>BalanceCheckDto</div>;
    //   break;
    case 'BalancePad':
      line = <div>BalancePadDto</div>;
      break;
    case 'Transaction':
      line = <TransactionPreview data={props.data as JournalTransactionItem} />;
      break;
  }
  return line;
}
