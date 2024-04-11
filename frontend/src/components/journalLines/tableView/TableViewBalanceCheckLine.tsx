import { JournalBalanceCheckItem } from '../../../rest-model';
import { ActionIcon, Badge, createStyles, getStylesRef, px } from '@mantine/core';
import { format } from 'date-fns';
import Amount from '../../Amount';
import BigNumber from 'bignumber.js';
import { IconZoomExclamation } from '@tabler/icons';
import { openContextModal } from '@mantine/modals';
import PayeeNarration from '../../basic/PayeeNarration';

const useStyles = createStyles((theme, _params, getRef) => ({
  payee: {
    fontWeight: 'bold',
  },
  narration: {},
  accumulated: {
    color: theme.colors.gray[7],
    fontFeatureSettings: 'tnum',
    fontSize: px(theme.fontSizes.sm) * 0.75,
  },
  positiveAmount: {
    color: theme.colors.gray[7],
    fontWeight: 'bold',
    fontFeatureSettings: 'tnum',
    fontSize: px(theme.fontSizes.sm) * 0.95,
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
  wrapper: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'end',
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
  data: JournalBalanceCheckItem;
}

export default function TableViewBalanceCheckLine({ data }: Props) {
  const { classes } = useStyles();

  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = (e: any) => {
    openContextModal({
      modal: 'transactionPreviewModal',
      title: 'Balance Check Detail',
      size: 'lg',
      centered: true,
      innerProps: {
        journalId: data.id,
      },
    });
  };
  const isBalanced = new BigNumber(data.postings[0].account_after_number).eq(new BigNumber(data.postings[0].account_before_number));
  return (
    <tr className={`${classes.actionHider} ${!isBalanced ? classes.notBalance : ''}`}>
      <td>{time}</td>
      <td>
        <Badge size="xs" variant="outline">
          Check
        </Badge>
      </td>
      <td>
        <PayeeNarration payee={data.payee} narration={data.narration} />
      </td>
      <td>
        <div className={classes.wrapper}>
          <div className={!isBalanced ? classes.negativeAmount : classes.positiveAmount}>
            <Amount amount={data.postings[0].account_after_number} currency={data.postings[0].account_after_commodity} />
          </div>
          {!isBalanced && (
            <span className={classes.accumulated}>
              accumulated: <Amount amount={data.postings[0].account_before_number} currency={data.postings[0].account_before_commodity} />
            </span>
          )}
        </div>
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
