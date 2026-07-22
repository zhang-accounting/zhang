import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import { Badge } from '../ui/badge';
import { JournalBalanceCheckItem } from '@/api/types';
interface Props {
  data: JournalBalanceCheckItem;
}

export default function BalanceCheckPreview(props: Props) {
  const isBalanced = new BigNumber(props.data.postings[0].account_after.number).eq(new BigNumber(props.data.postings[0].account_before.number));
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
            <Amount amount={checkInfo.account_after.number} currency={checkInfo.account_after.commodity} />
          </p>
        </DashLine>

        {!isBalanced && (
          <>
            <DashLine>
              <p className="line-clamp-1">Accumulated Amount</p>
              <p className="line-clamp-1">
                <Amount amount={checkInfo.account_before.number} currency={checkInfo.account_before.commodity} />
              </p>
            </DashLine>

            <DashLine>
              <p className="line-clamp-1">Distance</p>
              <p className="line-clamp-1">
                <Amount amount={checkInfo.inferred_unit.number} currency={checkInfo.inferred_unit.commodity} />
              </p>
            </DashLine>
          </>
        )}
      </Section>
    </div>
  );
}
