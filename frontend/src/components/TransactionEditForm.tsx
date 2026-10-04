import BigNumber from 'bignumber.js';
import { format } from 'date-fns';
import { useAtomValue } from 'jotai';
import { CalendarIcon, Plus, TableProperties, X } from 'lucide-react';
import { useEffect, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { optionValue, retrieveFiles, retrieveNewTransactionInfo, retrieveOptions } from '@/api/requests';
import { JournalTransactionItem, MetaEntry } from '@/api/types';
import { GroupCombobox } from '@/components/basic/GroupCombobox';
import { useDateFormat, useDateLocale } from '@/components/layout/use-date-format';
import { useListState } from '@/hooks/use-list-state';
import { cn } from '@/lib/utils';
import { accountSelectItemsAtom } from '../states/account';
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from './ui/accordion';
import { Button } from './ui/button';
import { Calendar } from './ui/calendar';
import { Field, FieldDescription, FieldGroup, FieldLabel } from './ui/field';
import { Input } from './ui/input';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import {
  directiveText,
  DOCUMENT_KEY,
  ledgerFormat,
  parseAmount,
  PostingDraft,
  toPostingDrafts,
  toPostingRequest,
  toRequestMetas,
  TransactionFormValue,
} from './transaction-form-utils';

export type { TransactionFormValue } from './transaction-form-utils';

interface Props {
  onChange(data: TransactionFormValue, isValid: boolean): void;
  data?: JournalTransactionItem;
}

const POSTING_CARD = 'grid grid-cols-[minmax(0,1fr)_auto_auto] gap-2 rounded-lg border p-2';
const POSTING_ROW = 'md:grid-cols-[minmax(0,1fr)_10rem_auto_auto] md:items-center md:rounded-none md:border-0 md:p-0';

/** Number of metadata entries on a posting's metadata toggle. */
const COUNT_BADGE = cn(
  'absolute top-0.5 right-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-0.5 md:-top-0.5 md:-right-0.5',
  'bg-foreground-2 text-[10px] leading-none font-medium text-background tabular-nums',
);

/** Touch-friendly control height (DESIGN.md: 40px on mobile, shadcn's dense 32px from md). */
const CONTROL = 'h-10 md:h-8';

const AMOUNT_ERROR: Record<'cost_price' | 'no_commodity' | 'invalid', string> = {
  cost_price: 'ledger.txn.amount_cost_price',
  no_commodity: 'ledger.txn.amount_no_commodity',
  invalid: 'ledger.txn.amount_invalid',
};

/** `yyyy-MM-dd HH:mm:ss` in the ledger timezone (the server converts the submitted instant to it before writing). */
function formatLedgerDateTime(date: Date, timeZone?: string) {
  if (timeZone) {
    try {
      const parts = Object.fromEntries(
        new Intl.DateTimeFormat('en-US', {
          timeZone,
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit',
          second: '2-digit',
          hourCycle: 'h23',
        })
          .formatToParts(date)
          .map((part) => [part.type, part.value]),
      );
      return `${parts.year}-${parts.month}-${parts.day} ${parts.hour}:${parts.minute}:${parts.second}`;
    } catch {
      // unknown timezone name: fall back to the browser's
    }
  }
  return format(date, 'yyyy-MM-dd HH:mm:ss');
}

/** Picking a day in the calendar keeps the time of day (an edit must not silently move a transaction to 00:00). */
function withTimeOf(day: Date, previous: Date | undefined) {
  if (!previous) return day;
  const next = new Date(day);
  next.setHours(previous.getHours(), previous.getMinutes(), previous.getSeconds(), previous.getMilliseconds());
  return next;
}

/** Key / value rows of a metadata editor (the transaction's or one posting's). */
function MetaRows({ metas, onChange }: { metas: MetaEntry[]; onChange(next: MetaEntry[]): void }) {
  const { t } = useTranslation();
  const setProp = (idx: number, prop: keyof MetaEntry, text: string) => onChange(metas.map((meta, i) => (i === idx ? { ...meta, [prop]: text } : meta)));
  return metas.map((meta, idx) => (
    <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)_auto] items-center gap-2" key={idx}>
      <Input
        className={CONTROL}
        aria-label={t('ledger.txn.meta_key')}
        placeholder={t('ledger.txn.meta_key')}
        // keys are case-sensitive identifiers (beancount wants a lowercase first letter): no iOS auto-capitalisation
        autoCapitalize="none"
        autoCorrect="off"
        autoComplete="off"
        spellCheck={false}
        value={meta.key}
        onChange={(e) => setProp(idx, 'key', e.target.value)}
      />
      <Input
        className={CONTROL}
        aria-label={t('ledger.txn.meta_value')}
        placeholder={t('ledger.txn.meta_value')}
        value={meta.value}
        onChange={(e) => setProp(idx, 'value', e.target.value)}
      />
      <Button
        variant="ghost"
        size="icon"
        className="size-10 text-muted-foreground md:size-8"
        aria-label={t('ledger.txn.remove_meta')}
        onClick={() => onChange(metas.filter((_, i) => i !== idx))}
      >
        <X />
      </Button>
    </div>
  ));
}

export default function TransactionEditForm(props: Props) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const locale = useDateLocale();
  const payeeListId = useId();

  const [date, setDate] = useState<Date | undefined>(props.data?.datetime ? new Date(props.data.datetime) : new Date());
  const [dateOpen, setDateOpen] = useState(false);
  const [payee, setPayee] = useState<string>(props.data?.payee ?? '');
  const [narration, setNarration] = useState(props.data?.narration ?? '');
  const [postings, postingsHandler] = useListState<PostingDraft>(toPostingDrafts(props.data?.postings));
  const [metas, metaHandler] = useListState<MetaEntry>((props.data?.metas ?? []).filter((meta) => meta.key !== DOCUMENT_KEY));
  // Posting metadata editors are collapsed by default; ids of the expanded postings.
  const [openPostingMetas, setOpenPostingMetas] = useState<ReadonlySet<number>>(() => new Set());

  const accountItems = useAtomValue(accountSelectItemsAtom);
  const { value: options } = useAsync(async () => {
    const options = (await retrieveOptions({})).data.data;
    return { operatingCurrency: optionValue(options, 'operating_currency'), timezone: optionValue(options, 'timezone') };
  }, []);
  const operatingCurrency = options?.operatingCurrency;
  const { value: payees } = useAsync(async () => (await retrieveNewTransactionInfo({})).data.data.payee, []);
  const { value: fileFormat } = useAsync(async () => ledgerFormat((await retrieveFiles({})).data.data), []);

  const parsed = useMemo(() => postings.map((it) => parseAmount(it.amount, operatingCurrency)), [postings, operatingCurrency]);
  const emptyAmounts = parsed.filter((it) => it.status === 'empty').length;
  const invalidAmount = parsed.some((it) => it.status !== 'empty' && it.status !== 'ok');
  const missingAccount = postings.some((it) => !it.account);
  const isValid = !!date && !missingAccount && emptyAmounts <= 1 && !invalidAmount;

  // Sum per commodity when every amount is explicit, to warn about unbalanced input before the server rejects it.
  const imbalance = useMemo(() => {
    if (emptyAmounts > 0 || invalidAmount) return [];
    const sums: Record<string, BigNumber> = {};
    parsed.forEach((it) => {
      if (it.status !== 'ok') return;
      sums[it.commodity] = (sums[it.commodity] ?? new BigNumber(0)).plus(it.number);
    });
    return Object.entries(sums).filter(([, sum]) => !sum.isZero());
  }, [parsed, emptyAmounts, invalidAmount]);

  // Exactly what is submitted; the preview below is rendered from this value.
  const value = useMemo<TransactionFormValue>(
    () => ({
      datetime: (date ?? new Date()).toISOString(),
      payee: payee ?? '',
      narration: narration,
      flag: props.data?.flag,
      postings: postings.map((it, idx) => toPostingRequest(it, parsed[idx])),
      tags: props.data?.tags ?? [],
      links: props.data?.links ?? [],
      metas: [...toRequestMetas(metas), ...(props.data?.metas ?? []).filter((meta) => meta.key === DOCUMENT_KEY)],
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [date, payee, narration, postings, metas, parsed],
  );

  useEffect(() => {
    props.onChange(value, isValid);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, isValid]);

  const preview = (): string =>
    directiveText(value, {
      datetime: formatLedgerDateTime(new Date(value.datetime), options?.timezone),
      format: fileFormat,
      amounts: parsed,
      invalidAmount: t('ledger.txn.preview_invalid_amount'),
      accountPlaceholder: t('ledger.txn.preview_account'),
    });

  const addPosting = () => {
    const id = Math.max(-1, ...postings.map((it) => it.id)) + 1;
    postingsHandler.append({ id, account: undefined, amount: '', metas: [], documents: [] });
  };

  /** Expands / collapses a posting's metadata editor; expanding an empty one starts with a blank row to fill in. */
  const togglePostingMetas = (idx: number) => {
    const posting = postings[idx];
    const open = openPostingMetas.has(posting.id);
    setOpenPostingMetas((current) => {
      const next = new Set(current);
      if (open) next.delete(posting.id);
      else next.add(posting.id);
      return next;
    });
    if (!open && posting.metas.length === 0) postingsHandler.setItemProp(idx, 'metas', [{ key: '', value: '' }]);
  };

  return (
    <div className="flex flex-col gap-6 pt-4 pb-1 md:pt-1">
      <FieldGroup className="grid gap-4 md:grid-cols-2">
        <Field>
          <FieldLabel htmlFor={`${payeeListId}-date`}>{t('ledger.txn.date')}</FieldLabel>
          <Popover open={dateOpen} onOpenChange={setDateOpen}>
            <PopoverTrigger
              id={`${payeeListId}-date`}
              render={<Button variant="outline" className={cn(CONTROL, 'w-full justify-start gap-2 font-normal', !date && 'text-muted-foreground')} />}
            >
              <CalendarIcon className="text-muted-foreground" />
              {date ? fmt.date(date) : t('ledger.txn.pick_date')}
            </PopoverTrigger>
            <PopoverContent className="w-auto p-0" align="start">
              <Calendar
                mode="single"
                locale={locale}
                selected={date}
                onSelect={(day) => {
                  setDate(day ? withTimeOf(day, date) : undefined);
                  setDateOpen(false);
                }}
                autoFocus
              />
            </PopoverContent>
          </Popover>
        </Field>
        <Field>
          <FieldLabel htmlFor={`${payeeListId}-payee`}>{t('ledger.txn.payee')}</FieldLabel>
          <Input
            id={`${payeeListId}-payee`}
            className={CONTROL}
            list={payeeListId}
            autoComplete="off"
            placeholder={t('ledger.txn.payee_placeholder')}
            value={payee}
            onChange={(e) => setPayee(e.target.value)}
          />
          <datalist id={payeeListId}>
            {(payees ?? []).map((it) => (
              <option key={it} value={it} />
            ))}
          </datalist>
        </Field>
        <Field className="md:col-span-2">
          <FieldLabel htmlFor={`${payeeListId}-narration`}>{t('ledger.txn.narration')}</FieldLabel>
          <Input
            id={`${payeeListId}-narration`}
            className={CONTROL}
            placeholder={t('ledger.txn.narration_placeholder')}
            value={narration}
            onChange={(e) => setNarration(e.target.value)}
          />
        </Field>
      </FieldGroup>

      <section className="flex flex-col gap-2" aria-label={t('ledger.txn.postings')}>
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-sm font-medium">{t('ledger.txn.postings')}</h3>
          <Button variant="ghost" size="sm" className="h-10 md:h-7" onClick={addPosting}>
            <Plus data-icon="inline-start" />
            {t('ledger.txn.add_posting')}
          </Button>
        </div>
        <div className="flex flex-col gap-2">
          {postings.map((posting, idx) => {
            const amount = parsed[idx];
            const amountError = amount.status === 'ok' || amount.status === 'empty' ? undefined : AMOUNT_ERROR[amount.status];
            const errorId = `${payeeListId}-posting-${posting.id}-error`;
            const metasId = `${payeeListId}-posting-${posting.id}-metas`;
            const metasOpen = openPostingMetas.has(posting.id);
            const metaCount = toRequestMetas(posting.metas).length;
            return (
              <div key={posting.id} className={cn(POSTING_CARD, POSTING_ROW)}>
                <GroupCombobox
                  className={cn(CONTROL, 'col-span-3 md:col-span-1')}
                  aria-label={t('ledger.txn.posting_account', { index: idx + 1 })}
                  placeholder={t('ledger.txn.account_placeholder')}
                  options={accountItems}
                  value={posting.account}
                  onChange={(e) => postingsHandler.setItemProp(idx, 'account', e)}
                />
                <Input
                  className={cn(CONTROL, 'tabular-nums')}
                  aria-label={t('ledger.txn.posting_amount', { index: idx + 1 })}
                  autoCapitalize="characters"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={operatingCurrency ? `0.00 ${operatingCurrency}` : t('ledger.txn.amount')}
                  aria-invalid={amountError ? true : undefined}
                  aria-describedby={amountError ? errorId : undefined}
                  value={posting.amount}
                  onChange={(e) => postingsHandler.setItemProp(idx, 'amount', e.target.value)}
                />
                <Button
                  variant="ghost"
                  size="icon"
                  className="relative size-10 text-muted-foreground md:size-8"
                  aria-label={t('ledger.txn.posting_metas_toggle', { index: idx + 1, count: metaCount })}
                  title={t('ledger.txn.posting_metas_title')}
                  aria-expanded={metasOpen}
                  aria-controls={metasOpen ? metasId : undefined}
                  onClick={() => togglePostingMetas(idx)}
                >
                  <TableProperties />
                  {metaCount > 0 && (
                    <span aria-hidden className={COUNT_BADGE}>
                      {metaCount}
                    </span>
                  )}
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-10 text-muted-foreground md:size-8"
                  aria-label={t('ledger.txn.remove_posting')}
                  disabled={postings.length <= 2}
                  onClick={() => {
                    postingsHandler.remove(idx);
                    setOpenPostingMetas((current) => new Set([...current].filter((id) => id !== posting.id)));
                  }}
                >
                  <X />
                </Button>
                {amountError && (
                  <p id={errorId} className="col-span-3 text-xs text-destructive md:col-span-4">
                    {t(amountError)}
                  </p>
                )}
                {metasOpen && (
                  <div
                    id={metasId}
                    role="group"
                    aria-label={t('ledger.txn.posting_metas', { index: idx + 1 })}
                    className="col-span-3 flex flex-col gap-2 border-t pt-2 md:col-span-4 md:mb-1 md:ml-3 md:border-t-0 md:border-l md:pt-0 md:pl-3"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-medium text-muted-foreground">{t('ledger.txn.posting_metas_title')}</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-10 md:h-7"
                        onClick={() => postingsHandler.setItemProp(idx, 'metas', [...posting.metas, { key: '', value: '' }])}
                      >
                        <Plus data-icon="inline-start" />
                        {t('ledger.txn.add_meta')}
                      </Button>
                    </div>
                    {posting.metas.length === 0 && <p className="text-xs text-muted-foreground">{t('ledger.txn.no_metas')}</p>}
                    <MetaRows metas={posting.metas} onChange={(next) => postingsHandler.setItemProp(idx, 'metas', next)} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {imbalance.length > 0 ? (
          <p className="text-xs text-destructive">
            {t('ledger.txn.unbalanced_by', { amount: imbalance.map(([commodity, sum]) => `${sum.toFormat()} ${commodity}`.trim()).join(', ') })}
          </p>
        ) : (
          <FieldDescription className="text-xs">{t('ledger.txn.postings_hint')}</FieldDescription>
        )}
      </section>

      <section className="flex flex-col gap-2" aria-label={t('ledger.txn.metas')}>
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-sm font-medium">{t('ledger.txn.metas')}</h3>
          <Button variant="ghost" size="sm" className="h-10 md:h-7" onClick={() => metaHandler.append({ key: '', value: '' })}>
            <Plus data-icon="inline-start" />
            {t('ledger.txn.add_meta')}
          </Button>
        </div>
        {metas.length === 0 && <p className="text-xs text-muted-foreground">{t('ledger.txn.no_metas')}</p>}
        <MetaRows metas={metas} onChange={metaHandler.setState} />
      </section>

      <Accordion>
        <AccordionItem value="preview" className="border-b-0">
          <AccordionTrigger className="py-2">{t('TXN_EDIT_PREVIEW')}</AccordionTrigger>
          <AccordionContent>
            <pre className="overflow-x-auto rounded-lg bg-muted p-3">
              <code className="font-mono text-xs break-words whitespace-pre-wrap">{preview()}</code>
            </pre>
          </AccordionContent>
        </AccordionItem>
      </Accordion>
    </div>
  );
}
