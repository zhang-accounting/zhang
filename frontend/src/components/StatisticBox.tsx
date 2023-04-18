import { useTranslation } from 'react-i18next';
import Amount from './Amount';
interface Props {
  text: string;
  amount: string;
  currency: string;
  detail?: any;
  negetive?: boolean;
}
export default function StatisticBox({ text, amount, currency, negetive }: Props) {
  const { t } = useTranslation();

  const displayBox = (
    <div>
      <div>{t(text)}</div>
      <Amount amount={amount} negetive={negetive} currency={currency} />
    </div>
  );
  return displayBox;
}
