import { format } from 'date-fns';
import { JournalBalancePadItem } from '../../rest-model';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import { Badge } from '../ui/badge';

interface Props {
  data: JournalBalancePadItem;
}

export default function BalancePadPreview(props: Props) {
  return (
    <div>
      <Section title="Check Info">
        <DashLine>
          <p className='line-clamp-1'>
            Datetime
          </p>
          <p className='line-clamp-1'>{format(new Date(props.data.datetime), 'yyyy-MM-dd HH:mm:ss')}</p>
        </DashLine>

        <DashLine>
          <p className='line-clamp-1'>
            Type
          </p>
          <p className='line-clamp-1'>Balance Pad</p>
        </DashLine>
        <DashLine>
          <p className='line-clamp-1'>
            Balance Account
          </p>
          <p className='line-clamp-1'>{props.data.postings[0].account}</p>
        </DashLine>
        <DashLine>
          <p className='line-clamp-1'>
            Pad Account
          </p>
          <p className='line-clamp-1'>{props.data.postings[1].account}</p>
        </DashLine>
        {(props.data.links || []).length > 0 && (
          <DashLine>
            <p className='line-clamp-1'>
              Links
            </p>
            <p className='line-clamp-1'>
              <div className='flex item-center mx-1 my-2 gap-2'>
                {(props.data.links || []).map((link) => (
                  <Badge key={link} variant="outline">
                    {link}
                  </Badge>
                ))}
              </div>
            </p>
          </DashLine>
         
        )}

        {(props.data.tags || []).length > 0 && (
          <DashLine>
            <p className='line-clamp-1'>
              Tags
            </p>
            <p className='line-clamp-1'>
              <div className='flex item-center mx-1 my-2 gap-2'>
                {(props.data.tags || []).map((tag) => (
                  <Badge key={tag}  variant="outline">
                    {tag}
                  </Badge>
                ))}
              </div>
            </p>
          </DashLine>
        )}
      </Section>
      <div className='mx-1 my-4'>
        <Section title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <DashLine key={idx}>
                <p className='line-clamp-1'>
                  {posting.account}
                </p>
                <div className="flex flex-col items-end">
                  <Amount amount={posting.inferred_unit_number} currency={posting.inferred_unit_commodity} />
                  <div className="text-sm text-gray-500">
                    Balance: <Amount amount={posting.account_after_number} currency={posting.account_after_commodity} />
                  </div>
                </div>
              </DashLine>
            ))}
          </>
        </Section>
      </div>

      {(props.data.metas ?? []).length > 0 && (
        <Section title="Metas">
          {props.data.metas
            .filter((meta) => meta.key !== 'document')
            .map((meta, idx) => (
              <DashLine key={idx}>
                <p className='line-clamp-1 my-xs'>
                  {meta.key}
                </p>
                <p className='line-clamp-1'>{meta.value}</p>
              </DashLine>
            ))}
        </Section>
      )}
    </div>
  );
}
