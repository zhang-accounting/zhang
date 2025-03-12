import { format } from 'date-fns';
import Amount from '../../Amount';
import BigNumber from 'bignumber.js';
import PayeeNarration from '../../basic/PayeeNarration';
import { cn } from '@/lib/utils';
import { useSetAtom } from 'jotai';
import { previewJournalAtom } from '@/states/journals';
import { Badge } from '@/components/ui/badge';
import { JournalBalanceCheckItem } from '@/api/types';
import { BadgeCheck, Scale } from 'lucide-react';

interface Props {
  data: JournalBalanceCheckItem;
}

export default function MobileViewBalanceCheckLine({ data }: Props) {
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const openPreviewModal = () => {
    setPreviewJournal(data);
  };

  const isBalanced = new BigNumber(data.postings[0].account_after_number).eq(new BigNumber(data.postings[0].account_before_number));

  return (
    <div className={cn('flex py-1 justify-between', !isBalanced && 'border-l-[3px] border-l-red-500')} onClick={openPreviewModal}>
      <div className="flex flex-col">
        <div className="flex items-center gap-2">
          <BadgeCheck className="w-4 h-4 text-primary" />
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
            className={cn('font-bold text-sm', !isBalanced && 'text-red-500')}
          />
          {!isBalanced && (
            <div className="text-sm text-gray-500">
              accumulated: <Amount
                amount={data.postings[0].account_before_number}
                currency={data.postings[0].account_before_commodity}
              />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
