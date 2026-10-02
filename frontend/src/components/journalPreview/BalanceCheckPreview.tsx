import { useTranslation } from 'react-i18next';
import { JournalBalanceCheckItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import Amount from '../Amount';
import { JournalTypeBadge } from '../journalLines/JournalBits';
import { isBalanceCheckPassed } from '../journalLines/journal-utils';
import { PreviewHeader, PreviewList, PreviewRow, PreviewSection } from './PreviewParts';

interface Props {
  data: JournalBalanceCheckItem;
}

export default function BalanceCheckPreview({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const isBalanced = isBalanceCheckPassed(data);
  const checkInfo = data.postings[0];

  return (
    <div className="flex flex-col gap-5 pt-4 pb-2 md:pt-0">
      <PreviewHeader
        badges={
          <>
            <JournalTypeBadge type="BalanceCheck" />
            {isBalanced ? (
              <Badge className="bg-emerald-500/15 text-emerald-700 dark:text-emerald-400">{t('ledger.preview.check_pass')}</Badge>
            ) : (
              <Badge variant="destructive">{t('ledger.journal.check_failed')}</Badge>
            )}
          </>
        }
        amount={<Amount amount={checkInfo.account_after.number} currency={checkInfo.account_after.commodity} />}
        title={checkInfo.account}
        meta={fmt.format(new Date(data.datetime), 'PP HH:mm:ss')}
      />
      <PreviewSection title={t('ledger.preview.check_info')}>
        <PreviewList>
          <PreviewRow label={t('ledger.preview.account')}>{checkInfo.account}</PreviewRow>
          <PreviewRow label={t('ledger.preview.balance_amount')}>
            <Amount amount={checkInfo.account_after.number} currency={checkInfo.account_after.commodity} />
          </PreviewRow>
          {!isBalanced && (
            <>
              <PreviewRow label={t('ledger.preview.accumulated_amount')}>
                <Amount amount={checkInfo.account_before.number} currency={checkInfo.account_before.commodity} />
              </PreviewRow>
              <PreviewRow label={t('ledger.preview.distance')}>
                <Amount className="text-destructive" amount={checkInfo.inferred_unit.number} currency={checkInfo.inferred_unit.commodity} />
              </PreviewRow>
            </>
          )}
        </PreviewList>
      </PreviewSection>
    </div>
  );
}
