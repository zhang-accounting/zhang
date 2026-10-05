import BigNumber from 'bignumber.js';
import { useSetAtom } from 'jotai';
import { ArrowLeft, ArrowRight, Pencil, ZoomIn } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { JournalBalanceCheckItem, JournalBalancePadItem, JournalItem, JournalTransactionItem } from '@/api/types';
import Amount from '@/components/Amount';
import { useDateFormat } from '@/components/layout/use-date-format';
import { cn } from '@/lib/utils';
import { editTransactionAtom, previewJournalAtom } from '@/states/journals';
import { calculate } from '@/utils/trx-calculator';
import { assertionOf } from './balance-assertion';
import { AssertionAmount } from './BalanceAssertion';
import { JournalChips, JournalStatusBadge, StatusEdge } from './JournalBits';
import { LineMenu } from './LineMenu';

type Posting = JournalTransactionItem['postings'][number];

/** `Assets:WeChat` → `WeChat`: the top-level type is implied by the flow arrow / context. */
const shortAccount = (account: string) => account.split(':').slice(1).join(':') || account;
const isOwnAccount = (account: string) => ['Assets', 'Liabilities'].includes(account.split(':')[0]);
const isNegative = (posting: Posting) => new BigNumber(posting.inferred_unit.number).isNegative();

/** 12px grey account chip (3px radius); the full name is in the tooltip. */
export function AccountChip({ account, className }: { account: string; className?: string }) {
  return (
    <span title={account} className={cn('min-w-0 truncate rounded-md bg-muted px-1.5 py-px text-xs text-foreground-2 md:max-w-64', className)}>
      {shortAccount(account)}
    </span>
  );
}

/**
 * Money flow of a transaction as two account chips: `own → destination` with a red arrow for money going out,
 * `own ← source` with a green arrow for income, a neutral arrow for transfers between own accounts. Further postings are
 * summarised as a `+N` chip.
 */
function FlowChips({ data, direction }: { data: JournalTransactionItem; direction: 'in' | 'out' | 'transfer' }) {
  const { t } = useTranslation();
  const sources = data.postings.filter(isNegative);
  const destinations = data.postings.filter((posting) => !isNegative(posting));
  let left: Posting | undefined;
  let right: Posting | undefined;
  if (direction === 'in') {
    left = destinations.find((posting) => isOwnAccount(posting.account)) ?? destinations[0];
    right = sources.find((posting) => !isOwnAccount(posting.account)) ?? sources[0];
  } else {
    left = sources.find((posting) => isOwnAccount(posting.account)) ?? sources[0];
    right = destinations.find((posting) => direction === 'transfer' || !isOwnAccount(posting.account)) ?? destinations[0];
  }
  const shown = [left, right].filter((posting): posting is Posting => posting !== undefined);
  const more = data.postings.length - shown.length;
  const Arrow = direction === 'in' ? ArrowLeft : ArrowRight;
  // One line that shrinks (chips truncate) instead of wrapping; tag chips and badges after it may wrap.
  return (
    <span className="flex max-w-full min-w-0 items-center gap-1.5">
      {left && <AccountChip account={left.account} className="max-w-[45%] shrink-0" />}
      {left && right && (
        <Arrow
          className={cn('size-3.5 shrink-0', direction === 'in' ? 'text-positive' : direction === 'out' ? 'text-negative' : 'text-muted-foreground')}
          aria-label={direction === 'in' ? t('ledger.journal.flow_from') : t('ledger.journal.flow_to')}
        />
      )}
      {right && <AccountChip account={right.account} />}
      {more > 0 && (
        <span
          className="shrink-0 rounded-md bg-muted px-1.5 py-px text-xs text-muted-foreground"
          title={data.postings.map((posting) => posting.account).join('\n')}
        >
          +{more}
        </span>
      )}
    </span>
  );
}

interface ShellProps {
  data: JournalItem;
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  chips: React.ReactNode;
  amount: React.ReactNode;
  meta?: React.ReactNode;
  menu: React.ReactNode;
  dense?: boolean;
  onOpen: () => void;
}

/**
 * Row anatomy (多少记账 style): line 1 narration + payee (muted), line 2 account / tag chips and status badges, amount on the
 * right with the time under it, then the "…" menu (always visible on touch screens, on hover / focus on desktop). The whole row
 * opens the preview.
 */
function RowShell({ data, title, subtitle, chips, amount, meta, menu, dense, onOpen }: ShellProps) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(event) => {
        // Only the row itself: Enter / Space on an inner control (tag chip, menu) keeps its own behaviour.
        if (event.target !== event.currentTarget) return;
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onOpen();
        }
      }}
      className={cn(
        'group/row relative flex w-full cursor-pointer items-center gap-2 py-2.5 pr-1.5 pl-3.5 text-left outline-none transition-colors md:gap-3 md:pr-2',
        dense && 'pr-3.5 md:pr-3.5',
        'hover:bg-accent/50 active:bg-accent focus-visible:bg-accent/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
        dense && 'py-2',
      )}
    >
      <StatusEdge data={data} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className={cn('min-w-0 shrink truncate', dense ? 'text-[13px]' : 'text-sm')}>{title}</span>
          {subtitle && <span className="min-w-0 shrink-[2] truncate text-[13px] text-muted-foreground">{subtitle}</span>}
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">{chips}</div>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-0.5">
        <div className={cn('flex flex-col items-end tabular-nums', dense ? 'text-sm' : 'text-[15px]')}>{amount}</div>
        {meta && <div className="text-xs text-muted-foreground tabular-nums">{meta}</div>}
      </div>
      {!dense && (
        <div className="shrink-0 md:opacity-0 md:group-hover/row:opacity-100 md:group-focus-within/row:opacity-100 md:has-[[aria-expanded=true]]:opacity-100">
          {menu}
        </div>
      )}
    </div>
  );
}

interface RowProps<T> {
  data: T;
  /** Show the day next to the time (lists that are not grouped by day, e.g. the overview's recent activity). */
  showDate?: boolean;
  /** Side cards (overview's recent activity): smaller type and no row menu (the preview it opens has the actions). */
  dense?: boolean;
}

function useWhen(datetime: string, showDate?: boolean) {
  const fmt = useDateFormat();
  return (showDate ? fmt.dayTime : fmt.time)(new Date(datetime));
}

function TransactionRow({ data, showDate, dense }: RowProps<JournalTransactionItem>) {
  const { t } = useTranslation();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const setEditTransaction = useSetAtom(editTransactionAtom);
  const when = useWhen(data.datetime, showDate);
  const summary = Array.from(calculate(data).values());
  const net = summary[0]?.number;
  const direction = net === undefined || net.isZero() ? 'transfer' : net.isPositive() ? 'in' : 'out';
  const transferred = data.postings.find(isNegative)?.inferred_unit;
  const openPreview = () => setPreviewJournal(data);

  return (
    <RowShell
      data={data}
      dense={dense}
      onOpen={openPreview}
      title={data.narration || data.payee || '—'}
      subtitle={data.narration ? data.payee : undefined}
      chips={
        <>
          <FlowChips data={data} direction={direction} />
          <JournalChips data={data} />
          <JournalStatusBadge data={data} />
        </>
      }
      amount={
        summary.length > 0 ? (
          summary.map((each) => <Amount key={each.commodity} tone amount={each.number} currency={each.commodity} />)
        ) : transferred ? (
          <Amount className="text-foreground-2" amount={new BigNumber(transferred.number).abs()} currency={transferred.commodity} />
        ) : (
          <span className="text-muted-foreground">—</span>
        )
      }
      meta={when}
      menu={
        <LineMenu
          actions={[
            { label: t('ledger.journal.edit'), icon: Pencil, onClick: () => setEditTransaction(data) },
            { label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreview },
          ]}
        />
      }
    />
  );
}

function BalanceCheckRow({ data, showDate, dense }: RowProps<JournalBalanceCheckItem>) {
  const { t } = useTranslation();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const when = useWhen(data.datetime, showDate);
  const assertion = assertionOf(data);
  const openPreview = () => setPreviewJournal(data);

  return (
    <RowShell
      data={data}
      dense={dense}
      onOpen={openPreview}
      title={data.narration || t('ledger.journal.balance_check_title')}
      subtitle={data.payee}
      chips={
        <>
          <span className="text-xs text-muted-foreground">{t('ledger.journal.type_check')}</span>
          {data.narration && <AccountChip account={data.narration} />}
          <JournalStatusBadge data={data} />
        </>
      }
      amount={assertion && <AssertionAmount assertion={assertion} withBalance />}
      meta={when}
      menu={<LineMenu actions={[{ label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreview }]} />}
    />
  );
}

function BalancePadRow({ data, showDate, dense }: RowProps<JournalBalancePadItem>) {
  const { t } = useTranslation();
  const setPreviewJournal = useSetAtom(previewJournalAtom);
  const when = useWhen(data.datetime, showDate);
  const posting = data.postings[0];
  const pad = data.postings[1];
  const openPreview = () => setPreviewJournal(data);

  return (
    <RowShell
      data={data}
      dense={dense}
      onOpen={openPreview}
      title={data.narration || t('ledger.journal.balance_pad_title')}
      subtitle={data.payee}
      chips={
        <>
          <span className="text-xs text-muted-foreground">{t('ledger.journal.type_pad')}</span>
          <AccountChip account={posting.account} />
          {pad && (
            <>
              <ArrowLeft className="size-3.5 shrink-0 text-muted-foreground" aria-label={t('ledger.journal.flow_from')} />
              <AccountChip account={pad.account} />
            </>
          )}
        </>
      }
      amount={<Amount amount={posting.account_after.number} currency={posting.account_after.commodity} />}
      meta={when}
      menu={<LineMenu actions={[{ label: t('ledger.journal.preview'), icon: ZoomIn, onClick: openPreview }]} />}
    />
  );
}

/** One journal (transaction, balance check or pad) as a clickable list row; see `RowShell` for the anatomy. */
export function JournalRow({ data, showDate, dense }: RowProps<JournalItem>) {
  switch (data.type) {
    case 'Transaction':
      return <TransactionRow data={data} showDate={showDate} dense={dense} />;
    case 'BalanceCheck':
      return <BalanceCheckRow data={data} showDate={showDate} dense={dense} />;
    case 'BalancePad':
      return <BalancePadRow data={data} showDate={showDate} dense={dense} />;
  }
  return null;
}
