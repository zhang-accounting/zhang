import { Group, Text } from '@mantine/core';
import { JournalItem, TransactionDto } from '../gql/jouralList';
import { IconArrowBigDownLines, IconArrowBigUpLines } from '@tabler/icons';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { calculate } from '../utils/trx-calculator';
interface Props {
  data: TransactionDto;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TransactionLine({ data, onClick }: Props) {
  const date = format(data.timestamp * 1000, 'yyyy-MM-dd');
  const time = format(data.timestamp * 1000, 'hh:mm:ss');
  const trClick = () => {
    console.log('clock');
    if (onClick) {
      onClick(data);
    }
  };
  const summary = calculate(data);
  return (
    <tr onClick={() => trClick()}>
      <td>
        <Text>{date}</Text>

        <Text size="xs" color="dimmed">
          {time}
        </Text>
      </td>
      <td>
        <Text lineClamp={1}>{data.narration}</Text>
        <Group position="apart">
          <Text mr={2} color="dimmed" size="xs">
            {data.payee}
          </Text>
          <Group position="right">
            {Array.from(summary.values()).map((each) => (
              <Group align="center" spacing="xs">
                {each.number.isPositive() ? <IconArrowBigDownLines stroke={1.5} /> : <IconArrowBigUpLines stroke={1.5} />}
                <span>
                  {each.number.abs().toFixed()} {each.currency}
                </span>
              </Group>
            ))}
          </Group>
        </Group>
      </td>
      <td>
        <Group spacing="xs" position="right">
          {/* <div className={classes.mainLinkInner}>
                        <IconArrowBigUpLines stroke={1.5}></IconArrowBigUpLines>
                        <Text>100.00 CNY</Text>
                    </div> */}
        </Group>
        {/* {data.postings.map((posting, idx) => (
                    <Group key={idx}>
                        <Text mx={2}>{posting?.account?.name}</Text>
                        <Text align={"right"} mx={2}>{posting.unit?.number} {posting.unit?.currency}</Text>
                    </Group>
                ))} */}
      </td>
    </tr>
  );
}
