import { ActionIcon, Group, Stack, Table } from '@mantine/core';
import { IconFile, IconPencil, IconZoomExclamation } from '@tabler/icons-react';
import { format } from 'date-fns';
import { JournalTransactionItem } from '../../../rest-model';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
import { openContextModal } from '@mantine/modals';
import PayeeNarration from '../../basic/PayeeNarration';
import { createStyles, getStylesRef } from '@mantine/emotion';
import { journalLinksAtom, journalTagsAtom } from '../../../states/journals';
import { useAtom } from 'jotai';
import { TableRow, TableCell } from '@/components/ui/table';
import { Badge } from '@/components/ui/badge';

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

  const [, setJournalTags] = useAtom(journalTagsAtom);
  const [, setJournalLinks] = useAtom(journalLinksAtom);

  const handleTagClick = (tag: string) => () => {
    setJournalTags((prevTags) => {
      if (prevTags.includes(tag)) {
        return prevTags;
      }
      return [...prevTags, tag];
    });
  };
  const handleLinkClick = (link: string) => () => {
    setJournalLinks((prevLinks) => {
      if (prevLinks.includes(link)) {
        return prevLinks;
      }
      return [...prevLinks, link];
    });
  };

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
    <TableRow className={` p-1 ${classes.actionHider} ${!data.is_balanced ? 'border-l-[3px] border-l-red-500' : ''} ${data.flag === '!' ? 'border-l-[3px] border-l-orange-500' : ''}`}>
      <TableCell>{time}</TableCell>
      <TableCell>
        <Badge color="gray" variant="outline">
          TRX
        </Badge>
      </TableCell>
      <TableCell>
        <Stack gap={'xs'}>
          <Group align="center" gap="xs">
            <PayeeNarration payee={data.payee} narration={data.narration} />
            {data.links &&
              data.links.map((it) => (
                <Badge key={it} className="cursor-pointer" color="blue" variant="secondary"  onClick={() => handleLinkClick(it)()}>
                  ^{it}
                </Badge>
              ))}
            {data.tags &&
              data.tags.map((tag) => (
                <Badge key={tag} className="cursor-pointer" color="blue" variant="secondary"  onClick={() => handleTagClick(tag)()}>
                  #{tag}
                </Badge>
              ))}
            {hasDocuments && <IconFile size="1rem" color={'gray'} stroke={1}></IconFile>}
          </Group>
        </Stack>
      </TableCell>
      <TableCell>
        {Array.from(summary.values()).map((each) => (
          <Group
            align="center"
            justify="right"
            gap="xs"
            key={each.currency}
            className={each.number.isPositive() ? classes.positiveAmount : classes.negativeAmount}
          >
            <Amount amount={each.number} currency={each.currency} />
          </Group>
        ))}
      </TableCell>
      <TableCell>
        <div className={classes.actions}>
          <ActionIcon color="gray" variant="white" size="sm" onClick={openEditModel}>
            <IconPencil size="1rem" />
          </ActionIcon>
          <ActionIcon color="gray" variant="white" size="sm" onClick={openPreviewModal}>
            <IconZoomExclamation size="1rem" />
          </ActionIcon>
        </div>
      </TableCell>
    </TableRow>
  );
}
