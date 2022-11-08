import { Group, Text, createStyles, Badge } from '@mantine/core';
import { JournalItem, TransactionDto } from '../gql/jouralList';
import { IconArrowBigDownLines, IconArrowBigUpLines } from '@tabler/icons';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { calculate } from '../utils/trx-calculator';

const useStyles = createStyles((theme) => ({
  positiveAmount: {
    color: theme.colors.green[8],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm,
  },
  negativeAmount: {
    color: theme.colors.red[8],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm,
  },
}));

interface Props {
  data: TransactionDto;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TransactionLine({ data, onClick }: Props) {
  const { classes } = useStyles();

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
        <Text lineClamp={1}>{data.narration || data.payee}</Text>
        <Group spacing="xs">
          {!!data.narration && (
            <Text mr={2} color="dimmed" size="xs">
              {data.payee}
            </Text>
          )}
          {data.links.map((link) => (
            <Badge key={link} size="xs" variant="dot">
              {link}
            </Badge>
          ))}
          {data.tags.map((tag) => (
            <Badge key={tag} color="orange" size="xs" variant="dot">
              {tag}
            </Badge>
          ))}
        </Group>
      </td>
      <td>
        <Group spacing="xs" position="right">
          {Array.from(summary.values()).map((each) => (
            <Group align="center" spacing="xs" className={each.number.isPositive() ? classes.positiveAmount : classes.negativeAmount}>
              {each.number.isPositive() ? <IconArrowBigDownLines stroke={1.5} /> : <IconArrowBigUpLines stroke={1.5} />}
              <span>
                {each.number.toFixed()} {each.currency}
              </span>
            </Group>
          ))}
        </Group>
      </td>
    </tr>
  );
}
