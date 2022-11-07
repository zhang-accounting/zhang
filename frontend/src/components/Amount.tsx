import { Text, Group, createStyles } from '@mantine/core';

const useStyles = createStyles((theme) => ({
  number: {
    fontFeatureSettings: '"tnum" 1',
  },
  postfix: {
    fontFeatureSettings: '"tnum" 1',
  },
}));

interface Props {
  amount?: string;
  currency?: string;
  negetive?: boolean;
}
export default function Amount({ amount, currency, negetive }: Props) {
  const { classes } = useStyles();
  const flag = negetive || false ? -1 : 1;
  var formatter = new Intl.NumberFormat('en-US', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 10,
  });
  const parsedValue = parseFloat(amount || '0');
  const value = parsedValue === 0 ? parsedValue : flag * parsedValue;
  return (
    <Group spacing={'xs'} position="right">
      <Text className={classes.number}>{formatter.format(value)}</Text>
      <Text mx={1} className={classes.postfix}>
        {currency}
      </Text>
    </Group>
  );
}
