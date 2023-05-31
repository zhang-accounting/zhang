import { createStyles, Text } from '@mantine/core';
import BigNumber from 'bignumber.js';
import { useAppSelector } from '../states';
import { getCommodityByName } from '../states/commodity';

const useStyles = createStyles((theme) => ({
  wrapper: {
    display: 'inline-flex',
    gap: theme.spacing.xs * 0.25,
  },
  number: {
    fontFeatureSettings: '"tnum" 1',
  },
  postfix: {
    fontFeatureSettings: '"tnum" 1',
  },
}));

interface Props {
  amount: string | number | BigNumber;
  currency: string;
  negetive?: boolean;
  mask?: boolean;
}

export default function Amount({ amount, currency, negetive, mask }: Props) {
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
  const displayedValue = value.toFormat(commodity?.precision ?? 2);
  const maskedValue = shouldMask ? displayedValue.replace(/\d/g, '*') : displayedValue;
  return (
    <span className={classes.wrapper}>
      {commodity?.prefix && (
        <Text mx={1} className={classes.postfix}>
          {commodity?.prefix}
        </Text>
      )}
      <Text className={classes.number}>{maskedValue}</Text>
      {commodity?.suffix && (
        <Text mx={1} className={classes.postfix}>
          {commodity?.suffix}
        </Text>
      )}
      {shouldDisplayCurrencyName && (
        <Text mx={1} className={classes.postfix}>
          {currency}
        </Text>
      )}
    </span>
  );
}
