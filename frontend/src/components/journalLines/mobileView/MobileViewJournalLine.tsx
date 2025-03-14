import { JournalItem } from '@/api/types';
import MobileViewBalancePadLine from './MobileViewBalancePadLine';
import MobileViewTransactionLine from './MobileViewTransactionLine';
import MobileViewBalanceCheckLine from './MobileViewBalanceCheckLine';

interface Props {
  data: JournalItem;
}
export default function MobileViewJournalLine({ data }: Props) {
  let line = null;
  switch (data.type) {
    case 'BalanceCheck':
      line = <MobileViewBalanceCheckLine data={data} />;
      break;
    case 'BalancePad':
      line = <MobileViewBalancePadLine data={data} />;
      break;
    case 'Transaction':
      line = <MobileViewTransactionLine data={data} />;
      break;
  }
  return line;
}
