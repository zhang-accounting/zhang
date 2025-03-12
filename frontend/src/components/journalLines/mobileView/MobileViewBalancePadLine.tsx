import { format } from 'date-fns';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { cn } from '@/lib/utils';
import { useSetAtom } from 'jotai';
import { previewJournalAtom } from '@/states/journals';
import { Badge } from '@/components/ui/badge';
import { JournalBalancePadItem } from '@/api/types';
import { Calculator, Scale } from 'lucide-react';

interface Props {
  data: JournalBalancePadItem;
}

export default function MobileViewBalancePadLine({ data }: Props) {
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = () => {
    setPreviewJournal(data);
  };

  return (
    <div className="flex py-1 justify-between gap-1" onClick={openPreviewModal}>
      <div className="flex flex-col">
        <div className="flex items-center gap-2">
          <Scale className="w-4 h-4" />
          <span className='line-clamp-1'>{data.narration ?? ''}</span>
        </div>

        <div className="flex items-center gap-2 px-6">
          {data.payee && <span className="text-sm">{data.payee}</span>}
          <span className="text-sm text-gray-500">{time}</span>
        </div>
      </div>

      <div>
        <div className="flex flex-col items-end gap-1 py-1">
          <Amount
            amount={data.postings[0].account_after_number}
            currency={data.postings[0].account_after_commodity}
            className="font-bold text-sm text-gray-700"
          />
        </div>
      </div>
    </div>
  );
}
