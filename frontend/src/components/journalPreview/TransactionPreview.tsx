import { format } from 'date-fns';
import Amount from '../Amount';
import DashLine from '../DashedLine';
import Section from '../Section';
import DocumentPreview from './DocumentPreview';
import AccountDocumentUpload from '../AccountDocumentUpload';
import { ImageLightBox } from '../ImageLightBox';
import { useState } from 'react';
import { Badge } from '@/components/ui/badge.tsx';
import { JournalTransactionItem } from '@/api/types';

interface Props {
  data: JournalTransactionItem;
}

export default function TransactionPreview(props: Props) {
  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);

  return (
    <div>
      <Section title="Transaction Info">
        <DashLine>
          <p className="line-clamp-1">Datetime</p>
          <p className="line-clamp-1">{format(new Date(props.data.datetime), 'yyyy-MM-dd HH:mm:ss')}</p>
        </DashLine>

        <DashLine>
          <p className="line-clamp-1">Type</p>
          <p className="line-clamp-1">Transaction</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Check Status</p>
          <p className="line-clamp-1">{props.data.is_balanced ? <Badge variant="outline">Pass</Badge> : <Badge color={'red'}>UNBALANCED</Badge>}</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Payee</p>
          <p className="line-clamp-1">{props.data.payee}</p>
        </DashLine>
        <DashLine>
          <p className="line-clamp-1">Narration</p>
          <p className="line-clamp-1">{props.data.narration}</p>
        </DashLine>

        {(props.data.links || []).length > 0 && (
          <DashLine>
            <p className="line-clamp-1">Links</p>
            <p className="line-clamp-1">
              <div className="flex items-center gap-2">
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
            <p className="line-clamp-1">Tags</p>
            <p className="line-clamp-1">
              <div className="flex items-center gap-2">
                {(props.data.tags || []).map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))}
              </div>
            </p>
          </DashLine>
        )}
      </Section>
      <div className="mx-1 my-4">
        <Section title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <DashLine key={idx}>
                <p className="line-clamp-1">{posting.account}</p>
                <div className={'flex flex-col items-end'}>
                  <Amount amount={posting.inferred_unit_number} currency={posting.inferred_unit_commodity} />
                  <div className={'text-sm text-gray-500'}>
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
          {(props.data.metas ?? [])
            .filter((meta) => meta.key !== 'document')
            .map((meta, idx) => (
              <DashLine key={idx}>
                <p className="line-clamp-1">{meta.key}</p>
                <p className="line-clamp-1">{meta.value}</p>
              </DashLine>
            ))}
        </Section>
      )}
      <div className="mx-1 my-4">
        <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
        <Section title={`${(props.data.metas ?? []).filter((meta) => meta.key === 'document').length} Documents`}>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 md:gap-6">
            {(props.data.metas ?? [])
              .filter((meta) => meta.key === 'document')
              .map((meta, idx) => (
                <DocumentPreview onClick={() => setLightboxSrc(meta.value)} key={idx} uri={meta.value} filename={meta.value} />
              ))}
            <AccountDocumentUpload id={props.data.id} type="transaction" />
          </div>
        </Section>
      </div>
    </div>
  );
}
