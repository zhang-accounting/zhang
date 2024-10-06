import { format } from 'date-fns';
import { Dispatch, SetStateAction } from 'react';
import { JournalBalancePadItem, JournalItem } from '../../../rest-model';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { TableRow, TableCell } from '@/components/ui/table';
import { cn } from '@/lib/utils';
import { useSetAtom } from 'jotai';
import { previewJournalAtom } from '@/states/journals';
import { LineMenu } from './LineMenu';
import { ZoomIn } from 'lucide-react';
import { Badge } from '@/components/ui/badge';


interface Props {
  data: JournalBalancePadItem;
  onClick?: Dispatch<SetStateAction<JournalItem | undefined>>;
}

export default function TableViewBalancePadLine({ data, onClick }: Props) {
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = () => {
    setPreviewJournal(data);
  };

  return (
    <TableRow className={cn(
      'p-1',
    )}>
      <TableCell>{time}</TableCell>
      <TableCell>
        <Badge variant="outline">
          Pad
        </Badge>
      </TableCell>
      <TableCell>
        <PayeeNarration payee={data.payee} narration={data.narration} />
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-2">
          <Amount 
            amount={data.postings[0].account_after_number} 
            currency={data.postings[0].account_after_commodity}
            className="font-bold text-gray-700 text-sm tabular-nums"
          />
        </div>

      </TableCell>
      <TableCell className="flex justify-end">
        <LineMenu actions={[{
          label: 'Preview',
          icon: ZoomIn,
          onClick: openPreviewModal,
        }]}/>
      </TableCell>
    </TableRow>
  );
}
