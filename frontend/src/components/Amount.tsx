import { createStyles, px, Text } from '@mantine/core';
import BigNumber from 'bignumber.js';
import { useAppSelector } from '../states';
import { getCommodityByName } from '../states/commodity';

const useStyles = createStyles((theme) => ({
  wrapper: {
    display: 'inline-flex',
    gap: px(theme.spacing.xs) * 0.25,
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
  negetive?: boolean;
  mask?: boolean;
  className?: string;
}

export default function Amount({ amount, currency, negetive, mask, className }: Props) {
  const { classes } = useStyles();
  const commodity = useAppSelector(getCommodityByName(currency));

  const flag = negetive || false ? -1 : 1;
  const shouldMask = mask || false;
  const shouldDisplayCurrencyName = !!!commodity?.prefix && !!!commodity?.suffix;

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
      {isNegative && (
        <Text mx={1} className={classes.part}>
          -
        </Text>
      )}
      <Text className={classes.number}>
        {commodity?.prefix}
        {maskedValue}
      </Text>
      {commodity?.suffix && (
        <Text mx={1} className={classes.part}>
          {commodity?.suffix}
        </Text>
      )}
      {shouldDisplayCurrencyName && (
        <Text mx={1} className={classes.part}>
          {currency}
        </Text>
      )}
    </span>
  );
}
