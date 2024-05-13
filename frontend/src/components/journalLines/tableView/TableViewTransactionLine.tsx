import { ActionIcon, Badge, Group, Table } from '@mantine/core';
import { IconFile, IconPencil, IconZoomExclamation } from '@tabler/icons-react';
import { format } from 'date-fns';
import { JournalTransactionItem } from '../../../rest-model';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
import { openContextModal } from '@mantine/modals';
import PayeeNarration from '../../basic/PayeeNarration';
import { createStyles, getStylesRef } from '@mantine/emotion';

const useStyles = createStyles((theme, _, u) => ({
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
    borderLeft: '3px solid red',
  },
  warning: {
    borderLeft: `3px solid ${theme.colors.orange[7]}`,
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
  data: JournalTransactionItem;
}

export default function TableViewTransactionLine({ data }: Props) {
  const { classes } = useStyles();

  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = (e: any) => {
    openContextModal({
      modal: 'transactionPreviewModal',
      title: 'Transaction Detail',
      size: 'lg',
      centered: true,
      innerProps: {
        journalId: data.id,
      },
    });
  };
  const openEditModel = (e: any) => {
    openContextModal({
      modal: 'transactionEditModal',
      title: 'Transaction Detail',
      size: 'lg',
      centered: true,
      innerProps: {
        data: data,
      },
    });
  };

  const summary = calculate(data);
  const hasDocuments = data.metas.some((meta) => meta.key === 'document');
  return (
    <Table.Tr className={`${classes.actionHider} ${!data.is_balanced ? classes.notBalance : ''} ${data.flag === '!' ? classes.warning : ''}`}>
      <Table.Td>{time}</Table.Td>
      <Table.Td>
        <Badge color="gray" size="xs" variant="outline">
          TRX
        </Badge>
      </Table.Td>
      <Table.Td>
        <Group align="center" gap="xs">
          <PayeeNarration payee={data.payee} narration={data.narration} />
          {hasDocuments && <IconFile size="1rem" color={'gray'} stroke={1}></IconFile>}
        </Group>
      </Table.Td>
      <Table.Td>
        {Array.from(summary.values()).map((each) => (
          <Group align="center" justify="right" gap="xs" className={each.number.isPositive() ? classes.positiveAmount : classes.negativeAmount}>
            <Amount amount={each.number} currency={each.currency} />
          </Group>
        ))}
      </Table.Td>
      <Table.Td>
        <div className={classes.actions}>
          <ActionIcon color="gray" variant="white" size="sm" onClick={openEditModel}>
            <IconPencil size="1rem" />
          </ActionIcon>
          <ActionIcon color="gray" variant="white" size="sm" onClick={openPreviewModal}>
            <IconZoomExclamation size="1rem" />
          </ActionIcon>
        </div>
      </Table.Td>
    </Table.Tr>
  );
}
