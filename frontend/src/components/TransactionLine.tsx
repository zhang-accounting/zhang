import { Group, Text, createStyles, Badge, Box, Grid } from '@mantine/core';
import { TransactionDto } from '../gql/jouralList';
import { IconArrowBigDownLines, IconArrowBigUpLines } from '@tabler/icons';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { calculate } from '../utils/trx-calculator';
import { JournalItem, JournalTransactionItem } from '../rest-model';

const useStyles = createStyles((theme) => ({
  payee: {
    fontWeight: "bold",
  },
  narration: {
    marginLeft: theme.spacing.xs*0.5,
    ":before": {
      content: '"·"',
      fontWeight: "bold",
      color: theme.colors.gray[5],
      marginRight: theme.spacing.xs*0.5,
    }
  },
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
  data: JournalTransactionItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TransactionLine({ data, onClick }: Props) {
  const { classes } = useStyles();

  const date = format(new Date(data.datetime), 'yyyy-MM-dd');
  const time = format(new Date(data.datetime), 'hh:mm:ss');
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
        <Grid align="center">
          <Grid.Col span={8}>
            <Box styles={{ maxWidth: '80%' }}>
              <Text lineClamp={1}>
                <span className={classes.payee}>{data.payee}</span>
                <span className={classes.narration}>{data.narration}</span>
              </Text>

              <Group spacing="xs">
                <Text mr={2} color="dimmed" size="xs">
                  {date} {time}
                </Text>

                {(data.links || []).map((link) => (
                  <Badge key={link} size="xs" variant="dot">
                    {link}
                  </Badge>
                ))}
                {(data.tags || []).map((tag) => (
                  <Badge key={tag} color="orange" size="xs" variant="dot">
                    {tag}
                  </Badge>
                ))}
              </Group>
            </Box>
          </Grid.Col>
          <Grid.Col span={4}>
            <Group align="center" spacing="xs" position="right">
              {Array.from(summary.values()).map((each) => (
                <Group align="center" spacing="xs" className={each.number.isPositive() ? classes.positiveAmount : classes.negativeAmount}>
                  <span>
                    {each.number.toFixed()} {each.currency}
                  </span>
                </Group>
              ))}
            </Group>
          </Grid.Col>
        </Grid>
      </td>
    </tr>
  );
}
