import { useTranslation } from 'react-i18next';
import Amount from './Amount';
import { CardContent, CardHeader } from './ui/card';
import { Card, CardTitle } from './ui/card';

interface Props {
  text: string;
  amount: string;
  currency?: string;
  detail?: any;
  negative?: boolean;
  hint?: string;
}

export default function StatisticBox({ text, amount, currency, negative }: Props) {
  const { t } = useTranslation();

  return (
    <Card className="rounded-sm">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium">{t(text)}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{currency ? <Amount amount={amount} negative={negative} currency={currency} /> : amount}</div>
        {/* {hint && <p className="text-xs text-muted-foreground">
                {hint}
              </p>} */}
      </CardContent>
    </Card>
  );
}
