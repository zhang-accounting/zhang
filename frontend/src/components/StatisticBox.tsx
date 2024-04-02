import { createStyles, px, Stack, Text } from '@mantine/core';
import { useTranslation } from 'react-i18next';
import Amount from './Amount';

const useStyles = createStyles((theme) => ({
  card: {
    backgroundColor: 'transparent',
    padding: theme.spacing.lg,
    border: `2px solid ${theme.colors[theme.primaryColor][6]}`,
    borderRadius: theme.radius.sm,
  },

  label: {
    fontFamily: `Greycliff CF, ${theme.fontFamily}`,
    lineHeight: 1,
    color: theme.colors.gray[9],
    fontSize: theme.fontSizes.sm,
    marginTop: 4,
  },

  lead: {
    fontWeight: 700,
    fontSize: px(theme.fontSizes.xl) * 1.15,
    lineHeight: 1,
    color: theme.colors.gray[9],
  },
}));

interface Props {
  text: string;
  amount: string;
  currency: string;
  detail?: any;
  negetive?: boolean;
  hint?: string;
}

export default function StatisticBox({ text, amount, currency, negetive, hint }: Props) {
  const { t } = useTranslation();
  const { classes } = useStyles();
  const displayBox = (
    <Stack mt="sm" spacing={'xs'} className={classes.card}>
      <Text className={classes.lead}>
        <Amount amount={amount} negetive={negetive} currency={currency} />
      </Text>
      <Text className={classes.label}>{t(text)}</Text>
    </Stack>
  );
  return displayBox;
}
