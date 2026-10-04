import { useSetAtom } from 'jotai';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { JournalTransactionItem } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { Badge } from '@/components/ui/badge';
import { journalFetcher } from '@/states/journals';
import { calculate } from '../../utils/trx-calculator';
import AccountDocumentUpload from '../AccountDocumentUpload';
import Amount from '../Amount';
import PayeeNarration from '../basic/PayeeNarration';
import { ImageLightBox } from '../ImageLightBox';
import { JournalStatusBadge, JournalTypeBadge } from '../journalLines/JournalBits';
import { transactionDocuments } from '../journalLines/journal-utils';
import { DOCUMENT_KEY } from '../transaction-form-utils';
import DocumentPreview from './DocumentPreview';
import { PostingRow, PreviewHeader, PreviewList, PreviewRow, PreviewSection } from './PreviewParts';

interface Props {
  data: JournalTransactionItem;
}

export default function TransactionPreview({ data }: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const refreshJournals = useSetAtom(journalFetcher);
  const [lightboxSrc, setLightboxSrc] = useState<string | undefined>(undefined);
  const summary = Array.from(calculate(data).values());
  const metas = (data.metas ?? []).filter((meta) => meta.key !== DOCUMENT_KEY);
  // the server links a posting's `document` to its transaction too: list them with the transaction's documents
  const documents = transactionDocuments(data);
  const tags = data.tags ?? [];
  const links = data.links ?? [];

  return (
    <div className="flex flex-col gap-5 pt-4 pb-2 md:pt-0">
      <PreviewHeader
        badges={
          <>
            <JournalTypeBadge type="Transaction" />
            <JournalStatusBadge data={data} />
            {data.flag && data.flag !== '*' && data.flag !== '!' && <Badge variant="outline">{data.flag}</Badge>}
          </>
        }
        amount={
          summary.length > 0 && (
            <span className="flex flex-col gap-0.5">
              {summary.map((each) => (
                <Amount key={each.commodity} tone signed amount={each.number} currency={each.commodity} />
              ))}
            </span>
          )
        }
        title={<PayeeNarration payee={data.payee} narration={data.narration} className="whitespace-normal" />}
        meta={fmt.weekdayDate(new Date(data.datetime)) + ' · ' + fmt.format(new Date(data.datetime), 'HH:mm:ss')}
      />

      <PreviewSection title={t('ledger.preview.postings')}>
        <PreviewList>
          {data.postings.map((posting, idx) => (
            <PostingRow
              key={idx}
              account={posting.account}
              metas={posting.metas.filter((meta) => meta.key !== DOCUMENT_KEY)}
              metasLabel={t('ledger.preview.posting_metas', { account: posting.account })}
              amount={<Amount amount={posting.inferred_unit.number} currency={posting.inferred_unit.commodity} />}
              balance={
                <>
                  {t('ledger.preview.balance_after')} <Amount amount={posting.account_after.number} currency={posting.account_after.commodity} />
                </>
              }
            />
          ))}
        </PreviewList>
      </PreviewSection>

      {(tags.length > 0 || links.length > 0 || metas.length > 0) && (
        <PreviewSection title={t('ledger.preview.details')}>
          <PreviewList>
            {links.length > 0 && (
              <PreviewRow label={t('ledger.preview.links')}>
                <span className="inline-flex flex-wrap justify-end gap-1">
                  {links.map((link) => (
                    <Badge key={link} variant="secondary">
                      ^{link}
                    </Badge>
                  ))}
                </span>
              </PreviewRow>
            )}
            {tags.length > 0 && (
              <PreviewRow label={t('ledger.preview.tags')}>
                <span className="inline-flex flex-wrap justify-end gap-1">
                  {tags.map((tag) => (
                    <Badge key={tag} variant="secondary">
                      #{tag}
                    </Badge>
                  ))}
                </span>
              </PreviewRow>
            )}
            {metas.map((meta, idx) => (
              <PreviewRow key={idx} label={meta.key}>
                {meta.value}
              </PreviewRow>
            ))}
          </PreviewList>
        </PreviewSection>
      )}

      <PreviewSection title={t('ledger.preview.documents', { count: documents.length })}>
        <ImageLightBox src={lightboxSrc} onChange={setLightboxSrc} />
        <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
          {documents.map((meta, idx) => (
            <DocumentPreview onClick={() => setLightboxSrc(meta.value)} key={idx} filename={meta.value} />
          ))}
          <AccountDocumentUpload id={data.id} type="transaction" onUploaded={() => refreshJournals()} />
        </div>
      </PreviewSection>
    </div>
  );
}
