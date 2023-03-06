import { Group, Stack, Text } from '@mantine/core';
import { IconCashBanknote, IconCashBanknoteOff } from '@tabler/icons';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
interface Props {

}
export default function BalanceCheckLine({  }: Props) {
  const date = format(0 * 1000, 'yyyy-MM-dd');
  const time = format(0 * 1000, 'hh:mm:ss');
  return (
    <div>hello </div>
  );
}
