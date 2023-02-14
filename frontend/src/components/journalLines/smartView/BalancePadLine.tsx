import { Badge, Box, createStyles, Grid, Group, Text } from '@mantine/core';
import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBlancePadItem, JournalItem } from '../../../rest-model';


const useStyles = createStyles((theme) => ({
  payee: {
    fontWeight: "bold",
  },
  narration: {
  },
  positiveAmount: {
    color: theme.colors.gray[8],
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm * 0.95,
  },
  negativeAmount: {
    color: theme.colors.red[8],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm,
  },
  notBalance: {
    borderLeft: "3px solid red"
  }
}));

interface Props {
  data: JournalBlancePadItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function BalancePadLine({ data, onClick }: Props) {
  const { classes } = useStyles();

  const date = format(new Date(data.datetime), 'yyyy-MM-dd');
  const time = format(new Date(data.datetime), 'hh:mm:ss');
  const trClick = () => {
    if (onClick) {
      onClick(data);
    }
  };

  const isBalanced = new BigNumber(data.postings[0].account_after_number) === new BigNumber(data.postings[0].account_before_number)
  return (
    <tr onClick={() => trClick()} className={!isBalanced ? classes.notBalance : ""}>
      <td>
        <Grid align="center">
          <Grid.Col span={8}>
            <Box styles={{ maxWidth: '80%' }}>
              <Text lineClamp={1}>
                <span className={classes.narration}>{data.postings[0].account}</span>
              </Text>


              <Group spacing="xs">
                <Text mr={2} color="dimmed" size="xs">
                  {time} Balance Pad
                </Text>
              </Group>
            </Box>
          </Grid.Col>
          <Grid.Col span={4}>
            <Group align="center" spacing="xs" position="right">
              <span className={isBalanced ? classes.positiveAmount : classes.negativeAmount}>
                {data.postings[0].account_after_number} {data.postings[0].account_after_commodity}
              </span>
              {!isBalanced &&
                <span className={classes.positiveAmount}>
                  current: {data.postings[0].account_before_number} {data.postings[0].account_before_commodity}
                </span>
              }
            </Group>
          </Grid.Col>
        </Grid>
      </td>
    </tr>
  );
}

