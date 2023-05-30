import { ActionIcon, Badge, createStyles, Group } from '@mantine/core';
import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBlancePadItem, JournalItem } from '../../../rest-model';
import Amount from '../../Amount';
import { IconZoomExclamation } from '@tabler/icons';
import { openContextModal } from '@mantine/modals';

const useStyles = createStyles((theme, _params, getRef) => ({
  payee: {
    fontWeight: 'bold',
  },
  narration: {},
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
    borderLeft: '3px solid red',
  },
  actionHider: {
    '&:hover': {
      [`& .${getRef('actions')}`]: {
        display: 'flex',
        alignItems: 'end',
        justifyContent: 'end',
      },
    },
  },
  actions: {
    ref: getRef('actions'),
    display: 'none',
  },
}));

interface Props {
  data: JournalBlancePadItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TableViewBalancePadLine({ data, onClick }: Props) {
  const { classes } = useStyles();

  const date = format(new Date(data.datetime), 'yyyy-MM-dd');
  const time = format(new Date(data.datetime), 'hh:mm:ss');

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
    <tr className={classes.actionHider}>
      <td>
        {date} {time}
      </td>
      <td>
        <Badge size="xs" variant="outline">
          Pad
        </Badge>
      </td>
      <td>{data.payee}</td>
      <td>{data.narration}</td>
      <td>
        <Group align="center" position="right" spacing="xs" className={classes.positiveAmount}>
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
      <td>
        <div className={classes.actions}>
          <ActionIcon size="sm" onClick={openPreviewModal}>
            <IconZoomExclamation size="1.125rem" />
          </ActionIcon>
        </div>
      </td>
    </tr>
  );
}
