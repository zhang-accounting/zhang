import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBalanceCheckItem, JournalItem } from '../../../rest-model';

interface Props {
  data: JournalBalanceCheckItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}
export default function BalanceCheckLine({}: Props) {
  const date = format(0 * 1000, 'yyyy-MM-dd');
  const time = format(0 * 1000, 'hh:mm:ss');
  return <div>hello </div>;
}
