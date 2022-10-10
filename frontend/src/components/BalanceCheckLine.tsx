import { Group, Stack, Text } from '@mantine/core';
import { IconCashBanknote, IconCashBanknoteOff } from '@tabler/icons';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { BalanceCheckDto, JournalItem } from '../gql/jouralList';
interface Props {
  data: BalanceCheckDto;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}
export default function BalanceCheckLine({ data, onClick }: Props) {
  const date = format(0 * 1000, 'yyyy-MM-dd');
  const time = format(0 * 1000, 'hh:mm:ss');
  const trClick = () => {
    console.log('clock');
    if (onClick) {
      onClick(data);
    }
  };
  return (
    <tr onClick={() => trClick()}>
      <td>
        <Text>{date}</Text>

        <Text size="xs" color="dimmed">
          {time}
        </Text>
      </td>
      <td>
        <Group position="apart">
          <div>
            <Text lineClamp={1}>{data.account?.name}</Text>
            <Text mr={2} color="dimmed" size="xs">
              Balance Check
            </Text>
          </div>
          <Stack align="flex-end" spacing="xs">
            {data.isBalanced ? (
              <Group spacing="xs" position="right">
                <IconCashBanknote stroke={1.5}></IconCashBanknote>
                <Text lineClamp={1}>
                  {data.currentAmount.number} {data.currentAmount.currency}
                </Text>
              </Group>
            ) : (
              <Group>
                <IconCashBanknoteOff stroke={1.5}></IconCashBanknoteOff>
                <Text lineClamp={1}>
                  {data.currentAmount.number} {data.currentAmount.currency}
                </Text>
              </Group>
            )}
            {!data.isBalanced && (
              <Text mr={2} color="dimmed" size="xs">
                {data.balanceAmount.number} {data.balanceAmount.currency}
              </Text>
            )}
          </Stack>
        </Group>
      </td>
      <td></td>
    </tr>
  );
}
