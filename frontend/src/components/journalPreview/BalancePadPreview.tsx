import { useTranslation } from 'react-i18next';
import { JournalBalancePadItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import Amount from '../Amount';
import { JournalTypeBadge } from '../journalLines/JournalBits';
import { PostingRow, PreviewHeader, PreviewList, PreviewRow, PreviewSection } from './PreviewParts';

interface Props {
  data: JournalBalancePadItem;
}

export default function BalancePadPreview({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const balance = data.postings[0];

  return (
    <div className="flex flex-col gap-5 pt-4 pb-2 md:pt-0">
      <PreviewHeader
        badges={<JournalTypeBadge type="BalancePad" />}
        amount={<Amount amount={balance.account_after.number} currency={balance.account_after.commodity} />}
        title={data.narration}
        meta={fmt.format(new Date(data.datetime), 'PP HH:mm:ss')}
      />
      <PreviewSection title={t('ledger.preview.pad_info')}>
        <PreviewList>
          <PreviewRow label={t('ledger.preview.balance_account')}>{data.postings[0]?.account}</PreviewRow>
          <PreviewRow label={t('ledger.preview.pad_account')}>{data.postings[1]?.account}</PreviewRow>
        </PreviewList>
      </PreviewSection>
      <PreviewSection title={t('ledger.preview.postings')}>
        <PreviewList>
          {data.postings.map((posting, idx) => (
            <PostingRow
              key={idx}
              account={posting.account}
              amount={<Amount amount={posting.inferred_unit.number} currency={posting.inferred_unit.commodity} />}
              balance={
                <>
                  {t('ledger.preview.balance_after')} <Amount amount={posting.account_after.number} currency={posting.account_after.commodity} />
                </>
              }
            />
          ))}
        </PreviewList>
      </PreviewSection>
    </div>
  );
}
