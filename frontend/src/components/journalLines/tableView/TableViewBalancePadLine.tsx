import { Badge, Box, createStyles, Grid, Group, Stack, Text } from '@mantine/core';
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

export default function TableViewBalancePadLine({ data, onClick }: Props) {
  const { classes } = useStyles();

  const date = format(new Date(data.datetime), 'yyyy-MM-dd');
  const time = format(new Date(data.datetime), 'hh:mm:ss');
  const trClick = () => {
    console.log('clock');
    if (onClick) {
      onClick(data);
    }
  };

  const isBalanced = new BigNumber(data.postings[0].account_after_number) === new BigNumber(data.postings[0].account_before_number)
  return (
    <tr className={!isBalanced ? classes.notBalance : ""}>
      <td>{date} {time}</td>
      <td>Pad</td>
      <td>{data.payee}</td>
      <td>{data.narration}</td>
      <td><Stack align="right" spacing="xs">
        <span className={isBalanced ? classes.positiveAmount : classes.negativeAmount}>
          {data.postings[0].account_after_number} {data.postings[0].account_after_commodity}
        </span>
        {!isBalanced &&
          <span className={classes.positiveAmount}>
            current: {data.postings[0].account_before_number} {data.postings[0].account_before_commodity}
          </span>
        }
      </Stack></td>
      <td></td>
    </tr>
  );
}

