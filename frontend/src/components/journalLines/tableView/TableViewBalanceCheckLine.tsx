import { useSetAtom } from 'jotai';
import { ZoomIn } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { JournalBalanceCheckItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { TableCell, TableRow } from '@/components/ui/table';
import { cn } from '@/lib/utils';
import { previewJournalAtom } from '@/states/journals';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { JournalStatusBadge, JournalTypeBadge, StatusEdge } from '../JournalBits';
import { isBalanceCheckPassed } from '../journal-utils';
import { LineMenu } from './LineMenu';

interface Props {
  data: JournalBalanceCheckItem;
}

export default function TableViewBalanceCheckLine({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const openPreviewModal = () => setPreviewJournal(data);
  const isBalanced = isBalanceCheckPassed(data);
  const posting = data.postings[0];

  return (
    <TableRow className="cursor-pointer" onClick={openPreviewModal}>
      <TableCell className="relative w-16 pl-4 text-xs text-muted-foreground tabular-nums">
        <StatusEdge data={data} />
        {fmt.time(new Date(data.datetime))}
      </TableCell>
      <TableCell className="w-24">
        <JournalTypeBadge type="BalanceCheck" />
      </TableCell>
      <TableCell className="w-full max-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <PayeeNarration payee={data.payee} narration={data.narration} />
          <JournalStatusBadge data={data} />
        </div>
        <div className="truncate text-xs text-muted-foreground">{posting.account}</div>
      </TableCell>
      <TableCell className="text-right">
        <div className="flex flex-col items-end gap-0.5">
          <Amount
            className={cn('font-medium', !isBalanced && 'text-destructive')}
            amount={posting.account_after.number}
            currency={posting.account_after.commodity}
          />
          {!isBalanced && (
            <span className="text-xs text-muted-foreground">
              {t('ledger.journal.accumulated')} <Amount amount={posting.account_before.number} currency={posting.account_before.commodity} />
            </span>
          )}
        </div>
      </TableCell>
      <TableCell className="w-12 pr-2 text-right">
        <LineMenu actions={[{ label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreviewModal }]} />
      </TableCell>
    </TableRow>
  );
}
