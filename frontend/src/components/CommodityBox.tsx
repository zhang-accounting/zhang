import Amount from './Amount';
import { Link } from 'react-router-dom';

interface Props {
  name: string;
  total_amount: string;
  operating_currency: boolean;
  latest_price_amount?: string;
  latest_price_commodity?: string;
  latest_price_date?: string;
}

export default function CommodityBox(props: Props) {
  return (
    <div className=" rounded-sm border-2 border-gray-100 bg-transparent hover:border-primary p-4">
      <div className="flex justify-between">
        <Link to={`/commodities/${props.name}`} className="text-lg">
          {props.name}
        </Link>
        <Amount amount={props.total_amount} currency="" className="text-lg" />
      </div>
      {props.latest_price_amount && (
        <div className="flex justify-between">
          <div></div>
          <div className="text-xs text-gray-500">
            <Amount amount={1} currency={props.name} /> = <Amount amount={props.latest_price_amount} currency={props.latest_price_commodity!} />
          </div>
        </div>
      )}
    </div>
  );
}
