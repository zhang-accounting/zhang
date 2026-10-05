import { CalendarIcon, Plus, TableProperties, X } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { optionValue, previewNewTransaction, previewTransactionUpdate, retrieveNewTransactionInfo, retrieveOptions } from '@/api/requests';
import { JournalTransactionItem, MetaEntry } from '@/api/types';
import { GroupCombobox } from '@/components/basic/GroupCombobox';
import { useDateFormat, useDateLocale } from '@/components/layout/use-date-format';
import { useListState } from '@/hooks/use-list-state';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { accountOptions } from '@/utils/account-options';
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from './ui/accordion';
import { Button } from './ui/button';
import { Calendar } from './ui/calendar';
import { Field, FieldDescription, FieldGroup, FieldLabel } from './ui/field';
import { Input } from './ui/input';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { calendarDay, LedgerDateTime, timeOfDay, withDay } from './ledger-datetime';
import { DOCUMENT_KEY, emptyDraft, PostingDraft, toPostingDrafts, toPostingRequest, toRequestMetas, TransactionFormValue } from './transaction-form-utils';
import { createPreviewer, fieldErrors, fieldErrorText, ledgerErrors, PreviewState, previewKey, refused, unbalancedText } from './transaction-preview';

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

/** The cost, price and comment fields of a posting's details; each is written as in a ledger file. */
type DetailField = 'cost' | 'price' | 'comment';
const DETAIL_FIELDS: DetailField[] = ['cost', 'price', 'comment'];

/** Whether a posting row has any detail besides its metadata: a cost, a price or a comment. */
const hasDetails = (posting: PostingDraft) => DETAIL_FIELDS.some((field) => posting[field].trim() !== '');

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

  // the ledger's wall-clock time, as the journal shows it and as it is sent: never an instant of the browser's timezone. A new
  // transaction is now by the ledger's clock, which the server tells
  const [datetime, setDatetime] = useState<LedgerDateTime | undefined>(props.data?.datetime);
  const date = datetime ? calendarDay(datetime) : undefined;
  const [dateOpen, setDateOpen] = useState(false);
  const [payee, setPayee] = useState<string>(props.data?.payee ?? '');
  const [narration, setNarration] = useState(props.data?.narration ?? '');
  const [postings, postingsHandler] = useListState<PostingDraft>(toPostingDrafts(props.data?.postings));
  const [metas, metaHandler] = useListState<MetaEntry>((props.data?.metas ?? []).filter((meta) => meta.key !== DOCUMENT_KEY));
  // Posting details (cost, price, comment, metadata) are collapsed by default, except for a posting that has a cost, a price
  // or a comment, which must stay in view when editing; ids of the expanded postings.
  const [openPostingMetas, setOpenPostingMetas] = useState<ReadonlySet<number>>(() => new Set(postings.filter(hasDetails).map((it) => it.id)));

  const { value: operatingCurrency } = useAsync(async () => optionValue((await retrieveOptions({})).data.data, 'operating_currency'), []);
  // the payees, and the accounts open at the transaction's date and time, by the rule the ledger checks it with
  const { value: info } = useAsync(async () => (await retrieveNewTransactionInfo({ datetime: datetime ?? null })).data.data, [datetime]);
  useEffect(() => {
    if (datetime === undefined && info) setDatetime(info.now);
  }, [datetime, info]);
  const payees = info?.payee;
  // an account a posting already uses stays in the list, so an edited transaction still shows a closed account
  const usedAccounts = useMemo(() => postings.map((it) => it.account), [postings]);
  const accountItems = useMemo(() => accountOptions(info?.account_name ?? [], usedAccounts), [info, usedAccounts]);

  // Exactly what is submitted; the server previews this value.
  const value = useMemo<TransactionFormValue>(
    () => ({
      datetime: datetime ?? '',
      payee: payee ?? '',
      narration: narration,
      flag: props.data?.flag,
      postings: postings.map(toPostingRequest),
      tags: props.data?.tags ?? [],
      links: props.data?.links ?? [],
      metas: [...toRequestMetas(metas), ...(props.data?.metas ?? []).filter((meta) => meta.key === DOCUMENT_KEY)],
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [datetime, payee, narration, postings, metas],
  );

  // The server reads the value as saving would, and tells what it would write, the fields it would refuse and what the ledger
  // would report, such as what the postings are unbalanced by once weighed by their cost or price: asked for shortly after the
  // last change, the newest answer kept.
  const transactionId = props.data?.id;
  const key = useMemo(() => previewKey(value), [value]);
  const [previewState, setPreviewState] = useState<PreviewState>();
  const previewer = useRef<ReturnType<typeof createPreviewer>>(undefined);
  useEffect(() => {
    const instance = createPreviewer(setPreviewState);
    previewer.current = instance;
    return () => instance.cancel();
  }, []);
  useEffect(() => {
    if (!value.datetime) return;
    previewer.current?.request(
      key,
      async () => (transactionId ? await previewTransactionUpdate({ ...value, transaction_id: transactionId }) : await previewNewTransaction(value)).data.data,
      apiErrorMessage,
    );
  }, [key, value, transactionId]);
  const preview = previewState?.preview;
  const unbalanced = unbalancedText(preview);
  const transactionErrors = Object.values(fieldErrors(previewState, key, null)).map((error) => fieldErrorText(error, t));

  const emptyAmounts = postings.filter((it) => it.amount.trim() === '').length;
  const missingAccount = postings.some((it) => !it.account);
  // one amount may be left empty, for booking to complete; a field the server refuses blocks saving until it is fixed
  const isValid = !!date && !missingAccount && emptyAmounts <= 1 && !refused(previewState, key);

  useEffect(() => {
    props.onChange(value, isValid);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, isValid]);

  const addPosting = () => {
    const id = Math.max(-1, ...postings.map((it) => it.id)) + 1;
    postingsHandler.append(emptyDraft(id));
  };

  /** Expands / collapses a posting's details; expanding one without metadata starts with a blank metadata row to fill in. */
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
                  if (day) setDatetime(withDay(day, datetime, timeOfDay(info?.now)));
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
            const errorId = `${payeeListId}-posting-${posting.id}-error`;
            const metasId = `${payeeListId}-posting-${posting.id}-metas`;
            const metasOpen = openPostingMetas.has(posting.id);
            const metaCount = toRequestMetas(posting.metas).length + DETAIL_FIELDS.filter((field) => posting[field].trim() !== '').length;
            const errors = fieldErrors(previewState, key, idx);
            // the cost and price errors show at their fields, in the details when they are open; an account not picked yet is
            // no error, the picker asks for it
            const accountError = posting.account ? errors.account : undefined;
            const postingErrors = [accountError, errors.unit, errors.metas, ...(metasOpen ? [] : [errors.cost, errors.price])]
              .filter((error) => error !== undefined)
              .map((error) => fieldErrorText(error, t));
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
                  aria-invalid={errors.unit ? true : undefined}
                  aria-describedby={postingErrors.length > 0 ? errorId : undefined}
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
                {postingErrors.length > 0 && (
                  <div id={errorId} className="col-span-3 flex flex-col gap-1 text-xs break-words text-destructive md:col-span-4">
                    {postingErrors.map((error, index) => (
                      <p key={index}>{error}</p>
                    ))}
                  </div>
                )}
                {metasOpen && (
                  <div
                    id={metasId}
                    role="group"
                    aria-label={t('ledger.txn.posting_metas', { index: idx + 1 })}
                    className="col-span-3 flex flex-col gap-2 border-t pt-2 md:col-span-4 md:mb-1 md:ml-3 md:border-t-0 md:border-l md:pt-0 md:pl-3"
                  >
                    <span className="text-xs font-medium text-muted-foreground">{t('ledger.txn.posting_metas_title')}</span>
                    <div className="grid gap-2 md:grid-cols-3">
                      {DETAIL_FIELDS.map((field) => {
                        const fieldId = `${payeeListId}-posting-${posting.id}-${field}`;
                        const error = field === 'comment' ? undefined : errors[field];
                        return (
                          <Field key={field}>
                            <FieldLabel htmlFor={fieldId} className="text-xs">
                              {t(`ledger.txn.${field}`)}
                            </FieldLabel>
                            <Input
                              id={fieldId}
                              className={cn(CONTROL, field !== 'comment' && 'tabular-nums')}
                              aria-label={t(`ledger.txn.posting_${field}`, { index: idx + 1 })}
                              autoCapitalize={field === 'comment' ? 'sentences' : 'characters'}
                              autoComplete="off"
                              spellCheck={field === 'comment'}
                              placeholder={t(`ledger.txn.${field}_placeholder`)}
                              aria-invalid={error ? true : undefined}
                              aria-describedby={error ? `${fieldId}-error` : undefined}
                              value={posting[field]}
                              onChange={(e) => postingsHandler.setItemProp(idx, field, e.target.value)}
                            />
                            {error && (
                              <p id={`${fieldId}-error`} className="text-xs break-words text-destructive">
                                {fieldErrorText(error, t)}
                              </p>
                            )}
                          </Field>
                        );
                      })}
                    </div>
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-medium text-muted-foreground">{t('ledger.txn.metas')}</span>
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
        {unbalanced ? (
          <p className="text-xs text-destructive">{t('ledger.txn.unbalanced_by', { amount: unbalanced })}</p>
        ) : (
          <FieldDescription className="text-xs">{t('ledger.txn.postings_hint')}</FieldDescription>
        )}
        {ledgerErrors(preview).map((error, index) => (
          <p key={index} className="text-xs text-destructive">
            {t(`ERROR.${error.error_type}`, { defaultValue: error.error_type, meta: error.metas })}
          </p>
        ))}
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
        {transactionErrors.map((error, index) => (
          <p key={index} className="text-xs break-words text-destructive">
            {error}
          </p>
        ))}
      </section>

      <Accordion>
        <AccordionItem value="preview" className="border-b-0">
          <AccordionTrigger className="py-2">{t('TXN_EDIT_PREVIEW')}</AccordionTrigger>
          <AccordionContent>
            <pre className="overflow-x-auto rounded-lg bg-muted p-3">
              <code className="font-mono text-xs break-words whitespace-pre-wrap">
                {preview?.text ??
                  previewState?.error ??
                  (missingAccount ? t('ledger.txn.preview_missing_account') : preview ? t('ledger.txn.preview_invalid') : '…')}
              </code>
            </pre>
          </AccordionContent>
        </AccordionItem>
      </Accordion>
    </div>
  );
}
