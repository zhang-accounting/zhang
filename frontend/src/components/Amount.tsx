import BigNumber from 'bignumber.js';
import { loadable_unwrap } from '../states';
import { commoditiesAtom } from '../states/commodity';
import { createStyles } from '@mantine/emotion';
import { selectAtom } from 'jotai/utils';
import { useAtomValue } from 'jotai';
import { useMemo } from 'react';

const useStyles = createStyles((theme, _, u) => ({
  wrapper: {
    display: 'inline-flex',
    gap: `calc(${theme.spacing.xs} * 0.25)`,
  },
  number: {
    fontFeatureSettings: '"tnum" 1',
  },
  part: {
    fontFeatureSettings: '"tnum" 1',
  },
}));

interface Props {
  amount: string | number | BigNumber;
  currency: string;
  negative?: boolean;
  mask?: boolean;
  className?: string;
}

export default function Amount({ amount, currency, negative, mask, className }: Props) {
  const { classes } = useStyles();
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
    <span className={`${classes.wrapper} ${className}`}>
      {isNegative && <span className={classes.part}>-</span>}
      <span className={classes.number}>
        {commodity?.prefix}
        {maskedValue}
      </span>
      {commodity?.suffix && <span className={classes.part}>{commodity?.suffix}</span>}
      {shouldDisplayCurrencyName && <span className={classes.part}>{currency}</span>}
    </span>
  );
}
