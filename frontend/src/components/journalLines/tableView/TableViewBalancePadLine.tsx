import { createStyles, Group } from '@mantine/core';
import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBlancePadItem, JournalItem } from '../../../rest-model';
import Amount from '../../Amount';


const useStyles = createStyles((theme) => ({
  payee: {
    fontWeight: "bold",
  },
  narration: {
  },
  positiveAmount: {
    color: theme.colors.gray[7],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm * 0.95,
  },
  negativeAmount: {
    color: theme.colors.red[5],
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
  

  const isBalanced = new BigNumber(data.postings[0].account_after_number) === new BigNumber(data.postings[0].account_before_number)
  return (
    <tr className={!isBalanced ? classes.notBalance : ""}>
      <td>{date} {time}</td>
      <td>Pad</td>
      <td>{data.payee}</td>
      <td>{data.narration}</td>
      <td>
      <Group align="center" position='right' spacing="xs" className={classes.positiveAmount }>
      <Amount amount={data.postings[0].account_after_number} currency={data.postings[0].account_after_commodity} />
        
        </Group>
        {/* <span className={isBalanced ? classes.positiveAmount : classes.negativeAmount}>
          <Amount amount={data.postings[0].account_after_number} currency={data.postings[0].account_after_commodity} />
        </span>
        {!isBalanced &&
          <div className={classes.positiveAmount}>
            current: <Amount amount={data.postings[0].account_before_number} currency={data.postings[0].account_before_commodity} />
          </div>
        } */}
      </td>
      <td></td>
    </tr>
  );
}

