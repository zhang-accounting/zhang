import BigNumber from 'bignumber.js';
import { useAtomValue } from 'jotai';
import { selectAtom } from 'jotai/utils';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';
import { amountTextParts, formatAmountText } from './amount-text';
import { loadable_unwrap } from '../states';
import { commoditiesAtom } from '../states/commodity';
import { defaultCommodityPrecisionAtom } from '../states/options';

interface Props {
  amount: string | number | BigNumber;
  currency: string;
  /** Flip the sign before displaying (e.g. income / liabilities are stored as negative numbers). */
  negative?: boolean;
  /** Replace every digit with `*`. */
  mask?: boolean;
  /**
   * Short notation (`¥444.7K`, `¥44.5万`) for |value| >= 10,000 (smaller values are shown in full: `¥18.01`, not `¥18`);
   * the full value is kept in the `title` attribute.
   */
  compact?: boolean;
  /** Colour by sign: positive → `text-positive`, negative → `text-negative`, zero → unchanged. */
  tone?: boolean;
  /** Prefix positive values with `+`. */
  signed?: boolean;
  /** Number only: no commodity prefix / suffix / name (dense lists where the currency is implied, e.g. sidebar accounts). */
  plain?: boolean;
  /** Every decimal the number has, at least the commodity's precision: a sub-cent difference shows instead of rounding to zero. */
  exact?: boolean;
  className?: string;
}

/** Semantic colour for a signed amount. Only use it for money moving in / out, never for decoration. */
function amountToneClass(value: BigNumber.Value) {
  const number = new BigNumber(value);
  if (number.isNaN() || number.isZero()) return '';
  return number.isPositive() ? 'text-positive' : 'text-negative';
}

/** Below this, compact notation would only drop precision without saving space. */
const COMPACT_THRESHOLD = 10_000;

/** Locale-aware compact number (`444.7K`, `44.5万`). */
function formatCompactNumber(value: BigNumber.Value, locale?: string) {
  const number = new BigNumber(value).toNumber();
  return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(number);
}

/** Money with the commodity's prefix / suffix / precision (`formatAmountText` writes it as text alike). Always tabular figures. */
export default function Amount({ amount, currency, negative, mask, compact, tone, signed, plain, exact, className }: Props) {
  const { i18n } = useTranslation();
  const commodity = useAtomValue(useMemo(() => selectAtom(commoditiesAtom, (val) => loadable_unwrap(val, undefined, (val) => val[currency])), [currency]));
  const defaultPrecision = useAtomValue(defaultCommodityPrecisionAtom);

  const flag = negative || false ? -1 : 1;
  const parsedValue = BigNumber.isBigNumber(amount) ? amount : new BigNumber(amount);
  const value = parsedValue.multipliedBy(flag);
  const options = { exact, plain, signed, defaultPrecision };
  const { sign, prefix, number: fullValue, suffix, currency: currencyName } = amountTextParts(value, currency, commodity, options);
  const useCompact = compact && value.abs().gte(COMPACT_THRESHOLD);
  const displayedValue = useCompact ? formatCompactNumber(value.abs(), i18n.language) : fullValue;
  const maskedValue = mask ? displayedValue.replace(/\d/g, '*') : displayedValue;
  const title = useCompact && !mask ? formatAmountText(value, currency, commodity, options) : undefined;

  return (
    <span className={cn('inline-flex items-baseline gap-1 whitespace-nowrap tabular-nums', tone && amountToneClass(value), className)} title={title}>
      <span>
        {sign}
        {prefix}
        {maskedValue}
        {suffix}
      </span>
      {currencyName !== undefined && <span className="text-[0.8em] font-normal opacity-70">{currencyName}</span>}
    </span>
  );
}
