import { useSetAtom } from 'jotai';
import { JournalBalanceCheckItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { cn } from '@/lib/utils';
import { previewJournalAtom } from '@/states/journals';
import Amount from '../../Amount';
import { isBalanceCheckPassed } from '../journal-utils';
import { MobileJournalRow } from './MobileJournalRow';

interface Props {
  showDate?: boolean;
  data: JournalBalanceCheckItem;
}

export default function MobileViewBalanceCheckLine({ data, showDate }: Props) {
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const posting = data.postings[0];
  const isBalanced = isBalanceCheckPassed(data);

  return (
    <MobileJournalRow
      data={data}
      title={data.narration || posting.account}
      subtitle={[posting.account, (showDate ? fmt.dateTime : fmt.time)(new Date(data.datetime))].join(' · ')}
      onOpen={() => setPreviewJournal(data)}
      trailing={
        <Amount
          className={cn('font-semibold', !isBalanced && 'text-destructive')}
          amount={posting.account_after.number}
          currency={posting.account_after.commodity}
        />
      }
    />
  );
}
