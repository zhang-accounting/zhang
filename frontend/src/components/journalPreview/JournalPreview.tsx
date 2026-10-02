import { JournalItem } from '@/api/types';
import BalanceCheckPreview from './BalanceCheckPreview';
import BalancePadPreview from './BalancePadPreview';
import TransactionPreview from './TransactionPreview';

interface Props {
  data?: JournalItem;
}

export default function JournalPreview({ data }: Props) {
  if (!data) return null;
  switch (data.type) {
    case 'BalanceCheck':
      return <BalanceCheckPreview data={data} />;
    case 'BalancePad':
      return <BalancePadPreview data={data} />;
    case 'Transaction':
      return <TransactionPreview data={data} />;
  }
  return null;
}
