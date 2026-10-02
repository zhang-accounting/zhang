import { useSetAtom } from 'jotai';
import { ZoomIn } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { JournalBalancePadItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { TableCell, TableRow } from '@/components/ui/table';
import { previewJournalAtom } from '@/states/journals';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { JournalTypeBadge } from '../JournalBits';
import { LineMenu } from './LineMenu';

interface Props {
  data: JournalBalancePadItem;
}

export default function TableViewBalancePadLine({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const openPreviewModal = () => setPreviewJournal(data);
  const posting = data.postings[0];

  return (
    <TableRow className="cursor-pointer" onClick={openPreviewModal}>
      <TableCell className="w-16 pl-4 text-xs text-muted-foreground tabular-nums">{fmt.time(new Date(data.datetime))}</TableCell>
      <TableCell className="w-24">
        <JournalTypeBadge type="BalancePad" />
      </TableCell>
      <TableCell className="w-full max-w-0">
        <PayeeNarration payee={data.payee} narration={data.narration} />
        <div className="truncate text-xs text-muted-foreground">{data.postings.map((it) => it.account).join(' ← ')}</div>
      </TableCell>
      <TableCell className="text-right">
        <Amount className="font-medium" amount={posting.account_after.number} currency={posting.account_after.commodity} />
      </TableCell>
      <TableCell className="w-12 pr-2 text-right">
        <LineMenu actions={[{ label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreviewModal }]} />
      </TableCell>
    </TableRow>
  );
}
