import { useTranslation } from 'react-i18next';
import { JournalBalanceCheckItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import Amount from '../Amount';
import { JournalTypeBadge } from '../journalLines/JournalBits';
import { assertionOf } from '../journalLines/balance-assertion';
import { AssertionDetails } from '../journalLines/BalanceAssertion';
import { isBalanceCheckPassed } from '../journalLines/journal-utils';
import { PreviewHeader, PreviewList, PreviewRow, PreviewSection } from './PreviewParts';

interface Props {
  data: JournalBalanceCheckItem;
}

export default function BalanceCheckPreview({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const assertion = assertionOf(data);
  const account = data.narration ?? '';

  return (
    <div className="flex flex-col gap-5 pt-4 pb-2 md:pt-0">
      <PreviewHeader
        badges={
          <>
            <JournalTypeBadge type="BalanceCheck" />
            {isBalanceCheckPassed(data) ? (
              <Badge className="bg-positive/10 text-positive">{t('ledger.preview.check_pass')}</Badge>
            ) : (
              <Badge variant="destructive">{t('ledger.journal.check_failed')}</Badge>
            )}
          </>
        }
        amount={<Amount amount={data.asserted.number} currency={data.asserted.commodity} />}
        title={account}
        meta={fmt.format(new Date(data.datetime), 'PP HH:mm:ss')}
      />
      <PreviewSection title={t('ledger.preview.check_info')}>
        <PreviewList>
          <PreviewRow label={t('ledger.preview.account')}>{account}</PreviewRow>
          {assertion && <AssertionDetails assertion={assertion} />}
        </PreviewList>
      </PreviewSection>
    </div>
  );
}
