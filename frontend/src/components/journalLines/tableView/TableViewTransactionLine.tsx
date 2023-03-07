import {Badge, createStyles, Group} from '@mantine/core';
import { IconFile } from "@tabler/icons";
import { format } from 'date-fns';
import { JournalTransactionItem } from '../../../rest-model';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';

const useStyles = createStyles((theme) => ({
  payee: {
    fontWeight: "bold",
  },
  narration: {
    // marginLeft: theme.spacing.xs*0.5,
  },
  positiveAmount: {
    color: theme.colors.green[8],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm,
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
  data: JournalTransactionItem;
}

export default function TableViewTransactionLine({ data }: Props) {
  const { classes } = useStyles();

  const date = format(new Date(data.datetime), 'yyyy-MM-dd');
  const time = format(new Date(data.datetime), 'hh:mm');

  const summary = calculate(data);
  const hasDocuments = data.metas.some(meta => meta.key === 'document');
  return (
    <tr className={!data.is_balanced ? classes.notBalance : ""}>
      <td>{date} {time}</td>
      <td><Badge color="gray" size="xs" variant="outline">TRX</Badge></td>
      <td>{data.payee}</td>
      <td>
        <Group align="center" spacing="xs">
          <span>{data.narration}</span>
          {hasDocuments &&
            <IconFile size={14} color={"gray"} stroke={1.5}></IconFile>
          }
        </Group>
      </td>
      <td>{Array.from(summary.values()).map((each) => (
        <Group align="center" position='right' spacing="xs" className={each.number.isPositive() ? classes.positiveAmount : classes.negativeAmount}>
          <Amount amount={each.number} currency={each.currency} />
        </Group>
      ))}</td>
      <td></td>
    </tr>
  );
}
