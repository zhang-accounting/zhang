import { JournalItem, TransactionDto } from '../../gql/jouralList';
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
    case 'BalanceCheckDto':
      line = <div>BalanceCheckDto</div>;
      break;
    case 'BalancePadDto':
      line = <div>BalancePadDto</div>;
      break;
    case 'TransactionDto':
      line = <TransactionPreview data={props.data as TransactionDto} />;
      break;
  }
  return line;
}
