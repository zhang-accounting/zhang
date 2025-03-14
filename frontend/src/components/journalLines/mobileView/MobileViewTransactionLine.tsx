import { JournalTransactionItem } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { format } from 'date-fns';
import { useAtom, useSetAtom } from 'jotai';
import { Files, ReceiptText } from 'lucide-react';
import { journalLinksAtom, journalTagsAtom, previewJournalAtom } from '../../../states/journals';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
interface Props {
  data: JournalTransactionItem;
}

export default function TableViewTransactionLine({ data }: Props) {
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const [, setJournalTags] = useAtom(journalTagsAtom);
  const [, setJournalLinks] = useAtom(journalLinksAtom);
  const setPreviewJournal = useSetAtom(previewJournalAtom);

  const handleTagClick = (tag: string) => () => {
    setJournalTags((prevTags) => {
      if (prevTags.includes(tag)) {
        return prevTags;
      }
      return [...prevTags, tag];
    });
  };
  const handleLinkClick = (link: string) => () => {
    setJournalLinks((prevLinks) => {
      if (prevLinks.includes(link)) {
        return prevLinks;
      }
      return [...prevLinks, link];
    });
  };

  const openPreviewModal = () => {
    setPreviewJournal(data);
  };

  const summary = calculate(data);
  const hasDocuments = data.metas.some((meta) => meta.key === 'document');
  return (
    <div
      onClick={openPreviewModal}
      className={cn(
        'flex py-1 justify-between',
        !data.is_balanced && 'border-l-[3px] border-l-red-500',
        data.flag === '!' && 'border-l-[3px] border-l-orange-500',
      )}
    >
      <div className="flex flex-col">
        <div className="flex items-center gap-2">
          <ReceiptText className="w-4 h-4 text-primary" />

          <span className="line-clamp-1">{data.narration ?? ''}</span>
          {data.links &&
            data.links.map((it) => (
              <Badge key={it} className="cursor-pointer" color="blue" variant="secondary" onClick={() => handleLinkClick(it)()}>
                ^{it}
              </Badge>
            ))}
          {data.tags &&
            data.tags.map((tag) => (
              <Badge key={tag} className="cursor-pointer" color="blue" variant="secondary" onClick={() => handleTagClick(tag)()}>
                #{tag}
              </Badge>
            ))}
          {hasDocuments && <Files className="w-4 h-4 text-gray-500" />}
        </div>

        <div className="flex items-center gap-2 px-6">
          {data.payee && <span className="text-sm">{data.payee}</span>}
          <span className="text-sm text-gray-500">{time}</span>
        </div>
      </div>

      <div>
        <div className="flex flex-col items-end gap-1 py-1">
          {Array.from(summary.values()).map((each) => (
            <Amount
              key={each.currency}
              className={cn('font-bold text-sm', each.number.isPositive() ? 'text-green-600' : 'text-red-500')}
              amount={each.number}
              currency={each.currency}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
