import { useSetAtom } from 'jotai';
import { JournalTransactionItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { previewJournalAtom } from '../../../states/journals';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
import { JournalChips } from '../JournalBits';
import { MobileJournalRow } from './MobileJournalRow';

interface Props {
  showDate?: boolean;
  data: JournalTransactionItem;
}

export default function MobileViewTransactionLine({ data, showDate }: Props) {
  const fmt = useDateFormat();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const summary = Array.from(calculate(data).values());
  const title = data.narration || data.payee || '—';
  const subtitle = [data.narration ? data.payee : null, (showDate ? fmt.dateTime : fmt.time)(new Date(data.datetime))].filter(Boolean).join(' · ');

  return (
    <MobileJournalRow
      data={data}
      title={title}
      subtitle={subtitle}
      extra={<JournalChips data={data} className="mt-1" />}
      onOpen={() => setPreviewJournal(data)}
      trailing={summary.map((each) => (
        <Amount key={each.commodity} className="font-semibold" tone amount={each.number} currency={each.commodity} />
      ))}
    />
  );
}
