import { Dispatch, SetStateAction } from 'react';
import { BalancePadDto, JournalItem } from '../gql/jouralList';

interface Props {
  data: BalancePadDto;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function BalancePadLine(props: Props) {
  return <div>BalancepadLine</div>;
}
