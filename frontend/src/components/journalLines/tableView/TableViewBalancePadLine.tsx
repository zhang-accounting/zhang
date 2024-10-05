import { ActionIcon, Badge, Group, Table } from '@mantine/core';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBalancePadItem, JournalItem } from '../../../rest-model';
import Amount from '../../Amount';
import { IconZoomExclamation } from '@tabler/icons-react';
import { openContextModal } from '@mantine/modals';
import PayeeNarration from '../../basic/PayeeNarration';
import { createStyles, getStylesRef } from '@mantine/emotion';
import { TableRow, TableCell } from '@/components/ui/table';

const useStyles = createStyles((theme, _, u) => ({
  payee: {
    fontWeight: 'bold',
  },
  narration: {},
  positiveAmount: {
    color: theme.colors.gray[7],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: `calc(${theme.fontSizes.sm} * 0.95)`,
  },
  negativeAmount: {
    color: theme.colors.red[5],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: theme.fontSizes.sm,
  },
  notBalance: {
    borderLeft: '3px solid red',
  },
  actionHider: {
    '&:hover': {
      [`& .${getStylesRef('actions')}`]: {
        display: 'flex',
        alignItems: 'end',
        justifyContent: 'end',
      },
    },
  },
  actions: {
    ref: getStylesRef('actions'),
    display: 'none',
  },
}));

interface Props {
  data: JournalBalancePadItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TableViewBalancePadLine({ data, onClick }: Props) {
  const { classes } = useStyles();

  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = (e: any) => {
    openContextModal({
      modal: 'transactionPreviewModal',
      title: 'Balance Pad Detail',
      size: 'lg',
      centered: true,
      innerProps: {
        journalId: data.id,
      },
    });
  };

  // const isBalanced = new BigNumber(data.postings[0].account_after_number) === new BigNumber(data.postings[0].account_before_number)
  return (
    <TableRow className={classes.actionHider}>
      <TableCell>{time}</TableCell>
      <TableCell>
        <Badge size="xs" variant="outline">
          Pad
        </Badge>
      </TableCell>
      <TableCell>
        <PayeeNarration payee={data.payee} narration={data.narration} />
      </TableCell>
      <TableCell>
        <Group align="center" justify="right" gap="xs" className={classes.positiveAmount}>
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
      </TableCell>
      <TableCell>
        <div className={classes.actions}>
          <ActionIcon color="gray" variant="white" size="sm" onClick={openPreviewModal}>
            <IconZoomExclamation size="1.125rem" />
          </ActionIcon>
        </div>
      </TableCell>
    </TableRow>
  );
}
