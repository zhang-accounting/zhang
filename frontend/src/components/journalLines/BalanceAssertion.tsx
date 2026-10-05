import BigNumber from 'bignumber.js';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';
import Amount from '../Amount';
import { PreviewRow } from '../journalPreview/PreviewParts';
import type { BalanceAssertion } from './balance-assertion';

/**
 * The figure of a balance assertion in a journal row, the same in the journal and in an account's journal: `=` and the asserted
 * amount, red when the assertion failed. With `withBalance`, for a journal without a balance column, a failed one shows the
 * balance it was checked against under it.
 */
export function AssertionAmount({ assertion, withBalance, className }: { assertion: BalanceAssertion; withBalance?: boolean; className?: string }) {
  const { t } = useTranslation();
  return (
    <>
      <span title={t('ledger.preview.balance_amount')} className={cn('text-muted-foreground', !assertion.passed && 'text-destructive', className)}>
        = <Amount amount={assertion.asserted.number} currency={assertion.asserted.commodity} />
      </span>
      {withBalance && !assertion.passed && (
        <span className="text-xs text-muted-foreground">
          {t('ledger.journal.accumulated')} <Amount amount={assertion.checked_balance.number} currency={assertion.checked_balance.commodity} />
        </span>
      )}
    </>
  );
}

/**
 * The details of a balance assertion, as rows of a preview list: the asserted amount, its tolerance, and when the balance it was
 * checked against differs from it (a check within its tolerance passes with such a balance), that balance and the difference.
 */
export function AssertionDetails({ assertion }: { assertion: BalanceAssertion }) {
  const { t } = useTranslation();
  const hasDifference = !new BigNumber(assertion.difference.number).isZero();
  return (
    <>
      <PreviewRow label={t('ledger.preview.balance_amount')}>
        <Amount exact amount={assertion.asserted.number} currency={assertion.asserted.commodity} />
      </PreviewRow>
      {assertion.tolerance && (
        <PreviewRow label={t('ledger.preview.tolerance')}>
          <Amount exact amount={assertion.tolerance} currency={assertion.asserted.commodity} />
        </PreviewRow>
      )}
      {hasDifference && (
        <>
          <PreviewRow label={t('ledger.preview.accumulated_amount')}>
            <Amount exact amount={assertion.checked_balance.number} currency={assertion.checked_balance.commodity} />
          </PreviewRow>
          <PreviewRow label={t('ledger.preview.distance')}>
            <Amount
              exact
              className={assertion.passed ? undefined : 'text-destructive'}
              amount={assertion.difference.number}
              currency={assertion.difference.commodity}
            />
          </PreviewRow>
        </>
      )}
    </>
  );
}
