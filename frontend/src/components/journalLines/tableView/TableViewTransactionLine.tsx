import { format } from 'date-fns';
import { calculate } from '../../../utils/trx-calculator';
import Amount from '../../Amount';
import PayeeNarration from '../../basic/PayeeNarration';
import { editTransactionAtom, journalLinksAtom, journalTagsAtom, previewJournalAtom } from '../../../states/journals';
import { useAtom, useSetAtom } from 'jotai';
import { TableRow, TableCell } from '@/components/ui/table';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { LineMenu } from './LineMenu';
import { Files, Pencil, ZoomIn } from 'lucide-react';
import { JournalTransactionItem } from '@/api/types';
interface Props {
  data: JournalTransactionItem;
}

export default function TableViewTransactionLine({ data }: Props) {
  const time = format(new Date(data.datetime), 'HH:mm:ss');

  const [, setJournalTags] = useAtom(journalTagsAtom);
  const [, setJournalLinks] = useAtom(journalLinksAtom);
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const setEditTransaction = useSetAtom(editTransactionAtom);

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

  const openEditModel = () => {
    setEditTransaction(data);
  };

  const summary = calculate(data);
  const hasDocuments = data.metas.some((meta) => meta.key === 'document');
  return (
    <TableRow className={cn('p-1', !data.is_balanced && 'border-l-[3px] border-l-red-500', data.flag === '!' && 'border-l-[3px] border-l-orange-500')}>
      <TableCell>{time}</TableCell>
      <TableCell>
        <Badge color="gray" variant="outline">
          TRX
        </Badge>
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2">
          <PayeeNarration payee={data.payee} narration={data.narration} onClick={openPreviewModal} />
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
      </TableCell>
      <TableCell className="">
        <div className="flex flex-col items-end">
          {Array.from(summary.values()).map((each) => (
            <Amount
              key={each.currency}
              className={cn('font-bold text-sm', each.number.isPositive() ? 'text-green-600' : 'text-red-500')}
              amount={each.number}
              currency={each.currency}
            />
          ))}
        </div>
      </TableCell>
      <TableCell className="flex justify-end">
        <LineMenu
          actions={[
            {
              label: 'Edit',
              icon: Pencil,
              onClick: openEditModel,
            },
            {
              label: 'Preview',
              icon: ZoomIn,
              onClick: openPreviewModal,
            },
          ]}
        />
      </TableCell>
    </TableRow>
  );
}
