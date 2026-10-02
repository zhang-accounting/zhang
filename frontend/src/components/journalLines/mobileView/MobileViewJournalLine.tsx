import { JournalItem } from '@/api/types';
import MobileViewBalanceCheckLine from './MobileViewBalanceCheckLine';
import MobileViewBalancePadLine from './MobileViewBalancePadLine';
import MobileViewTransactionLine from './MobileViewTransactionLine';

interface Props {
  data: JournalItem;
  /** Show the date next to the time (lists that are not grouped by day). */
  showDate?: boolean;
}

export default function MobileViewJournalLine({ data, showDate }: Props) {
  switch (data.type) {
    case 'BalanceCheck':
      return <MobileViewBalanceCheckLine data={data} showDate={showDate} />;
    case 'BalancePad':
      return <MobileViewBalancePadLine data={data} showDate={showDate} />;
    case 'Transaction':
      return <MobileViewTransactionLine data={data} showDate={showDate} />;
  }
  return null;
}
