import BigNumber from 'bignumber.js';
import { loadable_unwrap } from '../states';
import { commoditiesAtom } from '../states/commodity';
import { selectAtom } from 'jotai/utils';
import { useAtomValue } from 'jotai';
import { useMemo } from 'react';



interface Props {
  amount: string | number | BigNumber;
  currency: string;
  negative?: boolean;
  mask?: boolean;
  className?: string;
}

export default function Amount({ amount, currency, negative, mask, className }: Props) {
  
  const commodity = useAtomValue(useMemo(() => selectAtom(commoditiesAtom, (val) => loadable_unwrap(val, undefined, (val) => val[currency])), [currency]));

  const flag = negative || false ? -1 : 1;
  const shouldMask = mask || false;
  const shouldDisplayCurrencyName = !commodity?.prefix && !commodity?.suffix;

  let parsedValue: BigNumber;
  if (typeof amount === 'string') {
    parsedValue = new BigNumber(amount);
  } else if (typeof amount === 'number') {
    parsedValue = new BigNumber(amount);
  } else {
    parsedValue = amount;
  }

  const value = parsedValue.multipliedBy(flag);
  const isNegative = !value.isZero() && value.isNegative();
  const displayedValue = value.abs().toFormat(commodity?.precision ?? 2);
  const maskedValue = shouldMask ? displayedValue.replace(/\d/g, '*') : displayedValue;
  return (
    <span className={`inline-flex gap-[calc(theme.spacing.xs*0.25)] ${className}`}>
      {isNegative && <span className="font-feature-settings-tnum">-</span>}
      <span className="font-feature-settings-tnum">
        {commodity?.prefix}
        {maskedValue}
      </span>
      {commodity?.suffix && <span className="font-feature-settings-tnum">{commodity?.suffix}</span>}
      {shouldDisplayCurrencyName && <span className="font-feature-settings-tnum">{currency}</span>}
    </span>
  );
}
