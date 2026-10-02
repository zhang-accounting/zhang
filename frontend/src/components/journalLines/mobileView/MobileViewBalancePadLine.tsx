import { useSetAtom } from 'jotai';
import { JournalBalancePadItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { previewJournalAtom } from '@/states/journals';
import Amount from '../../Amount';
import { MobileJournalRow } from './MobileJournalRow';

interface Props {
  showDate?: boolean;
  data: JournalBalancePadItem;
}

export default function MobileViewBalancePadLine({ data, showDate }: Props) {
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const posting = data.postings[0];

  return (
    <MobileJournalRow
      data={data}
      title={posting.account}
      subtitle={[data.postings[1]?.account, (showDate ? fmt.dateTime : fmt.time)(new Date(data.datetime))].filter(Boolean).join(' · ')}
      onOpen={() => setPreviewJournal(data)}
      trailing={<Amount className="font-semibold" amount={posting.account_after.number} currency={posting.account_after.commodity} />}
    />
  );
}
