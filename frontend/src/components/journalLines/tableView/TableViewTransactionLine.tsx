import { useSetAtom } from 'jotai';
import { FileCode, Pencil, ZoomIn } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { JournalTransactionItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { TableCell, TableRow } from '@/components/ui/table';
import { editTransactionAtom, previewJournalAtom } from '../../../states/journals';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { JournalChips, JournalStatusBadge, JournalTypeBadge, StatusEdge } from '../JournalBits';
import { accountsFlow, RAW_EDIT_URI, transactionEditBlocker } from '../journal-utils';
import { LineMenu } from './LineMenu';

interface Props {
  data: JournalTransactionItem;
}

export default function TableViewTransactionLine({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const setEditTransaction = useSetAtom(editTransactionAtom);
  const navigate = useNavigate();
  const editBlocked = transactionEditBlocker(data) !== null;

  const openPreviewModal = () => setPreviewJournal(data);
  const openEditModel = () => setEditTransaction(data);
  const summary = Array.from(calculate(data).values());

  return (
    <TableRow className="cursor-pointer" onClick={openPreviewModal}>
      <TableCell className="relative w-16 pl-4 text-xs text-muted-foreground tabular-nums">
        <StatusEdge data={data} />
        {fmt.time(new Date(data.datetime))}
      </TableCell>
      <TableCell className="w-24">
        <JournalTypeBadge type="Transaction" />
      </TableCell>
      <TableCell className="w-full max-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <PayeeNarration payee={data.payee} narration={data.narration} />
          <JournalStatusBadge data={data} />
          <JournalChips data={data} className="shrink-0" />
        </div>
        <div className="truncate text-xs text-muted-foreground">{accountsFlow(data)}</div>
      </TableCell>
      <TableCell className="text-right">
        <div className="flex flex-col items-end gap-0.5">
          {summary.length === 0 && <span className="text-muted-foreground">—</span>}
          {summary.map((each) => (
            <Amount key={each.commodity} className="font-medium" tone amount={each.number} currency={each.commodity} />
          ))}
        </div>
      </TableCell>
      <TableCell className="w-12 pr-2 text-right">
        <LineMenu
          actions={[
            editBlocked
              ? { label: t('ledger.journal.edit'), icon: Pencil, onClick: () => undefined, disabled: true, hint: t('ledger.txn.edit_blocked_short') }
              : { label: t('ledger.journal.edit'), icon: Pencil, onClick: openEditModel },
            ...(editBlocked ? [{ label: t('ledger.txn.open_raw_edit'), icon: FileCode, onClick: () => navigate(RAW_EDIT_URI) }] : []),
            { label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreviewModal },
          ]}
        />
      </TableCell>
    </TableRow>
  );
}
