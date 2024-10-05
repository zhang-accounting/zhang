import DividerWithAction from './basic/DividerWithAction';
import { useListState } from '@mantine/hooks';
import { useEffect, useState } from 'react';
import useSWR from 'swr';
import { InfoForNewTransaction, JournalTransactionItem } from '../rest-model';
import { useTranslation } from 'react-i18next';
import { format } from 'date-fns';
import { accountSelectItemsAtom } from '../states/account';
import { useAtomValue } from 'jotai/index';
import { fetcher } from '../global.ts';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover.tsx';
import { Button } from './ui/button.tsx';
import { cn } from '@/lib/utils.ts';
import { CalendarIcon, CirclePlus, X } from 'lucide-react';
import { Calendar } from './ui/calendar.tsx';
import { Input } from './ui/input.tsx';
import { Combobox } from './ui/combobox.tsx';
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from './ui/accordion.tsx';

interface Posting {
  account: string | undefined;
  amount: string;
}

interface SelectItem {
  label: string;
  value: string;
  group?: string;
}


interface Props {
  onChange(data: any, isValid: boolean): void;

  data?: JournalTransactionItem;
}

export default function TransactionEditForm(props: Props) {
  console.log("transaction edit form", props.data);
  const { t } = useTranslation();

  const { data, error } = useSWR<InfoForNewTransaction>('/api/for-new-transaction', fetcher);

  const [dateOnly] = useState(true);
  const [date, setDate] = useState<Date | undefined>(props.data?.datetime ? new Date(props.data?.datetime) : new Date());
  const [payee, setPayee] = useState<string | undefined>(props.data?.payee ?? undefined);
  const [narration, setNarration] = useState(props.data?.narration ?? '');
  const [postings, postingsHandler] = useListState<Posting>(
    props.data?.postings?.map((item) => ({
      account: item.account ?? undefined,
      amount: `${item.unit_number ?? ''} ${item.unit_commodity ?? ''}`.trim(),
    })) ?? [
      { account: undefined, amount: '' },
      { account: undefined, amount: '' },
    ],
  );

  const [metas, metaHandler] = useListState<{ key: string; value: string }>(props.data?.metas ?? []);

  const [payeeSelectItems, setPayeeSelectItems] = useState<SelectItem[]>([]);
  const accountItems = useAtomValue(accountSelectItemsAtom);
  useEffect(() => {
    const newPayeeSelectItems: SelectItem[] = (data?.payee ?? []).map((item) => {
      return {
        label: item,
        value: item,
      };
    });
    setPayeeSelectItems(newPayeeSelectItems);
  }, [data, setPayeeSelectItems]);

  useEffect(() => {
    props.onChange(
      {
        datetime: date?.toISOString(),
        payee: payee ?? '',
        narration: narration,
        postings: postings.map((it) => {
          let splitAmount = it.amount.trim().split(' ');
          return {
            account: it.account,
            unit:
              splitAmount[0] === ''
                ? null
                : {
                  number: splitAmount[0],
                  commodity: splitAmount[1],
                },
          };
        }),
        tags: [],
        links: [],
        metas: metas,
      },
      valid(),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [date, payee, narration, postings, metas]);

  const preview = (): string => {
    const dateDisplay = format(date || 0, dateOnly ? 'yyyy-MM-dd' : 'yyyy-MM-dd HH:mm:ss');
    const narrationDisplay = narration.trim().length === 0 ? '' : ` ${JSON.stringify(narration.trim())}`;
    const postingDisplay = postings.map((posting) => `  ${posting.account} ${posting.amount}`).join('\n');
    const metaDisplay = metas
      .filter((meta) => meta.key.trim() !== '' && meta.value.trim() !== '')
      .map((meta) => `  ${JSON.stringify(meta.key)}: ${JSON.stringify(meta.value)}`)
      .join('\n');
    return `${dateDisplay} ${JSON.stringify(payee || '')}${narrationDisplay}\n${postingDisplay}\n${metaDisplay}`;
  };

  const valid = (): boolean => {
    return postings.every((posting) => posting.account !== null) && postings.filter((posting) => posting.amount.trim().length === 0).length <= 1;
  };

  if (error) return <div>failed to load</div>;
  if (!data) return <div>loading...</div>;
  return (
    <div className='my-4'>
      <div className='grid grid-cols-12 gap-4'>
        <div className='col-span-12 lg:col-span-12'>
          <Popover>
            <PopoverTrigger asChild>
              <Button
                variant={"outline"}
                className={cn(
                  "w-full justify-start text-left font-normal",
                  !date && "text-muted-foreground"
                )}
              >
                <CalendarIcon className="mr-2 h-4 w-4" />
                {date ? format(date, "PPP") : <span>Pick a date</span>}
              </Button>
            </PopoverTrigger>
            <PopoverContent className="w-auto p-0">
              <Calendar
                mode="single"
                selected={date}
                onSelect={setDate}
                initialFocus
              />
            </PopoverContent>
          </Popover>
        </div>
        <div className='col-span-12 lg:col-span-6'>
          <Input className='w-full' placeholder="Payee" value={payee} onChange={(e) => setPayee(e.target.value)} />
        </div>
        <div className='col-span-12 lg:col-span-6'>
          <Input className='w-full' placeholder="Narration" value={narration} onChange={(e) => setNarration(e.target.value)} />
        </div>
      </div>

      <DividerWithAction
        value="Postings"
        icon={<CirclePlus className='w-4 h-4' />}
        onActionClick={() =>
          postingsHandler.append({
            account: undefined,
            amount: '',
          })
        }
      ></DividerWithAction>

      {postings.map((posting, idx) => (
        <div className='flex items-center gap-2 py-2' key={idx}>
          <Combobox
            options={accountItems}
            value={posting.account}
            onChange={(e) => postingsHandler.setItemProp(idx, 'account', e)}
          />

          <Input placeholder="Amount" value={posting.amount}
            onChange={(e) => postingsHandler.setItemProp(idx, 'amount', e.target.value)} />
          <Button variant="ghost" size="icon"
            disabled={postings.length <= 2}
            onClick={() => postingsHandler.remove(idx)}>
            <X className='w-4 h-4' />
          </Button>
        </div>
      ))}

      <DividerWithAction
        value="Metas"
        icon={<CirclePlus className='w-4 h-4' />}
        onActionClick={() => {
          metaHandler.append({ key: '', value: '' });
        }}
      ></DividerWithAction>

      {metas.map((meta, idx) => (
        <div className='flex items-center gap-2 py-2' key={idx}>
          <Input placeholder="key" value={meta.key}
            onChange={(e) => metaHandler.setItemProp(idx, 'key', e.target.value)} />
          <Input placeholder="value" value={meta.value}
            onChange={(e) => metaHandler.setItemProp(idx, 'value', e.target.value)} />
          <Button variant="ghost" size="icon" onClick={() => metaHandler.remove(idx)}>
            <X className='w-4 h-4' />
          </Button>
        </div>
      ))}

      <Accordion type="single" collapsible>
        <AccordionItem value="preview">
          <AccordionTrigger>{t('TXN_EDIT_PREVIEW')}</AccordionTrigger>
          <AccordionContent>
            <pre className="bg-gray-100 p-4 rounded-md overflow-x-auto">
              <code className="text-sm font-mono whitespace-pre-wrap break-words">
                {preview()}
              </code>
            </pre>
          </AccordionContent>
        </AccordionItem>
      </Accordion>
    </div>
  );
}
