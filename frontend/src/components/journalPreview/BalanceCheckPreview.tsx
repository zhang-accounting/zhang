import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { JournalBalanceCheckItem } from '../../rest-model';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import { Badge } from '../ui/badge';

interface Props {
  data: JournalBalanceCheckItem;
}

export default function BalanceCheckPreview(props: Props) {
  const isBalanced = new BigNumber(props.data.postings[0].account_after_number).eq(new BigNumber(props.data.postings[0].account_before_number));
  const checkInfo = props.data.postings[0];
  return (
    <div>
      <Section title="Check Info">
        <DashLine>
          <p className="line-clamp-1">Datetime</p>
          <p className="line-clamp-1">{format(new Date(props.data.datetime), 'yyyy-MM-dd HH:mm:ss')}</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Account</p>
          <p className="line-clamp-1">{checkInfo.account}</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Check Status</p>
          <p className="line-clamp-1">{isBalanced ? <Badge color={'green'}>Pass</Badge> : <Badge color={'red'}>UNBALANCED</Badge>}</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Balance Amount</p>
          <p className="line-clamp-1">
            <Amount amount={checkInfo.account_after_number} currency={checkInfo.account_after_commodity} />
          </p>
        </DashLine>

        {!isBalanced && (
          <>
            <DashLine>
              <p className="line-clamp-1">Accumulated Amount</p>
              <p className="line-clamp-1">
                <Amount amount={checkInfo.account_before_number} currency={checkInfo.account_before_commodity} />
              </p>
            </DashLine>

            <DashLine>
              <p className="line-clamp-1">Distance</p>
              <p className="line-clamp-1">
                <Amount amount={checkInfo.inferred_unit_number} currency={checkInfo.inferred_unit_commodity} />
              </p>
            </DashLine>
          </>
        )}
      </Section>
    </div>
  );
}
