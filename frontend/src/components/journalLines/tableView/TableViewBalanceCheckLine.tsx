import { JournalBalanceCheckItem } from '../../../rest-model';
import { format } from 'date-fns';
import Amount from '../../Amount';
import BigNumber from 'bignumber.js';
import PayeeNarration from '../../basic/PayeeNarration';
import { TableRow, TableCell } from '@/components/ui/table';
import { cn } from '@/lib/utils';
import { useSetAtom } from 'jotai';
import { previewJournalAtom } from '@/states/journals';
import { LineMenu } from './LineMenu';
import { ZoomIn } from 'lucide-react';
import { Badge } from '@/components/ui/badge';


interface Props {
  data: JournalBalanceCheckItem;
}

export default function TableViewBalanceCheckLine({ data }: Props) {
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = (e: any) => {
    setPreviewJournal(data);
  };
  const isBalanced = new BigNumber(data.postings[0].account_after_number).eq(new BigNumber(data.postings[0].account_before_number));
  return (
    <TableRow className={cn(
      'p-1',
      !isBalanced && 'border-l-[3px] border-l-red-500',
    )}>
      <TableCell>{time}</TableCell>
      <TableCell>
        <Badge variant="outline">
          Check
        </Badge>
      </TableCell>
      <TableCell>
        <PayeeNarration payee={data.payee} narration={data.narration} />
      </TableCell>
      <TableCell>

        <div className="flex flex-col items-end">
          <div className={cn(
            'font-bold text-gray-700 text-sm tabular-nums',
            !isBalanced && 'text-red-500'
          )}>
            <Amount amount={data.postings[0].account_after_number} currency={data.postings[0].account_after_commodity} />
          </div>
          {!isBalanced && (
            <span className="text-gray-700 text-sm tabular-nums">
              accumulated: <Amount amount={data.postings[0].account_before_number} currency={data.postings[0].account_before_commodity} />
            </span>
          )}
        </div>
      </TableCell>
      <TableCell className="flex justify-end">
        <LineMenu actions={[{
          label: 'Preview',
          icon: ZoomIn,
          onClick: () => openPreviewModal(data),
        }]} />  
      </TableCell>
    </TableRow>
  );
}
