// React-free, so `node --test` runs it (amount-text.test.ts); `Amount` and `useAmountText` read the commodities and options.
import BigNumber from 'bignumber.js';

/** What writing an amount needs of its commodity (`/api/commodities`): its precision, prefix and suffix. */
export interface CommodityFormat {
  precision?: number | null;
  prefix?: string | null;
  suffix?: string | null;
}

/** zhang's built-in `default_commodity_precision`: the precision of a commodity the ledger does not declare. */
export const DEFAULT_COMMODITY_PRECISION = 2;

export interface AmountTextOptions {
  /** The precision of a commodity the ledger does not declare: the ledger's `default_commodity_precision`. */
  defaultPrecision?: number;
  /** Every decimal the number has, at least the commodity's precision: a sub-cent difference shows instead of rounding to zero. */
  exact?: boolean;
  /** Number only: no commodity prefix, suffix or name. */
  plain?: boolean;
  /** Prefix positive values with `+`. */
  signed?: boolean;
}

/** The parts of an amount as `Amount` shows it; `formatAmountText` joins them. */
export interface AmountTextParts {
  /** `-` for a negative value, `+` for a positive one when `signed`, else empty */
  sign: string;
  prefix: string;
  /** the absolute value with thousands separators, at the commodity's precision */
  number: string;
  suffix: string;
  /** the commodity's name, written after the number when the commodity has neither a prefix nor a suffix */
  currency?: string;
}

/** The parts of `amount` of `currency`, written with the commodity's precision, prefix and suffix. */
export function amountTextParts(
  amount: BigNumber.Value,
  currency: string,
  commodity: CommodityFormat | undefined,
  { defaultPrecision = DEFAULT_COMMODITY_PRECISION, exact, plain, signed }: AmountTextOptions = {},
): AmountTextParts {
  const value = new BigNumber(amount);
  const precision = commodity?.precision ?? defaultPrecision;
  const isNegative = !value.isZero() && value.isNegative();
  return {
    sign: isNegative ? '-' : signed && !value.isZero() ? '+' : '',
    prefix: plain ? '' : (commodity?.prefix ?? ''),
    number: value.abs().toFormat(exact ? Math.max(precision, value.decimalPlaces() ?? 0) : precision),
    suffix: plain ? '' : (commodity?.suffix ?? ''),
    currency: plain || commodity?.prefix || commodity?.suffix ? undefined : currency,
  };
}

/** `amount` of `currency` as text, written as `Amount` shows it (`-¥1,234.57`, `1,234.57 USD`): for a tooltip or a message. */
export function formatAmountText(amount: BigNumber.Value, currency: string, commodity: CommodityFormat | undefined, options?: AmountTextOptions): string {
  const { sign, prefix, number, suffix, currency: name } = amountTextParts(amount, currency, commodity, options);
  return `${sign}${prefix}${number}${suffix}${name ? ` ${name}` : ''}`;
}
