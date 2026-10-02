import { useSetAtom } from 'jotai';
import { BadgeCheck, Files, type LucideIcon, ReceiptText, Scale } from 'lucide-react';
import * as React from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { JournalItem, JournalTransactionItem } from '@/api/types';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { journalKeywordAtom, journalLinksAtom, journalPageAtom, journalTagsAtom } from '@/states/journals';
import { hasDocuments, journalStatus } from './journal-utils';

type JournalType = JournalItem['type'];

const TYPE_ICON: Record<JournalType, LucideIcon> = {
  Transaction: ReceiptText,
  BalancePad: Scale,
  BalanceCheck: BadgeCheck,
};

const TYPE_LABEL: Record<JournalType, string> = {
  Transaction: 'ledger.journal.type_transaction',
  BalancePad: 'ledger.journal.type_pad',
  BalanceCheck: 'ledger.journal.type_check',
};

export function JournalTypeBadge({ type, className }: { type: JournalType; className?: string }) {
  const { t } = useTranslation();
  return (
    <Badge variant="outline" className={cn('font-normal text-muted-foreground', className)}>
      {t(TYPE_LABEL[type])}
    </Badge>
  );
}

/** Round icon tile used by the mobile rows; tinted when the journal has a problem. */
export function JournalTypeIcon({ type, status = 'ok', className }: { type: JournalType; status?: 'ok' | 'warning' | 'error'; className?: string }) {
  const Icon = TYPE_ICON[type];
  return (
    <span
      className={cn(
        'flex size-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground',
        status === 'error' && 'bg-destructive/10 text-destructive',
        status === 'warning' && 'bg-amber-500/15 text-amber-600 dark:text-amber-400',
        className,
      )}
      aria-hidden
    >
      <Icon className="size-4" />
    </span>
  );
}

/** "Unbalanced" / "Flagged" / "Check failed" badges; renders nothing for healthy journals. */
export function JournalStatusBadge({ data, className }: { data: JournalItem; className?: string }) {
  const { t } = useTranslation();
  const status = journalStatus(data);
  if (status === 'ok') return null;
  if (status === 'warning') {
    return <Badge className={cn('bg-amber-500/15 text-amber-700 dark:text-amber-400', className)}>{t('ledger.journal.flagged')}</Badge>;
  }
  return (
    <Badge variant="destructive" className={className}>
      {data.type === 'BalanceCheck' ? t('ledger.journal.check_failed') : t('ledger.journal.unbalanced')}
    </Badge>
  );
}

/** Thin coloured bar on the leading edge of a row that has a problem. */
export function StatusEdge({ data }: { data: JournalItem }) {
  const status = journalStatus(data);
  if (status === 'ok') return null;
  return <span aria-hidden className={cn('absolute inset-y-1.5 left-0 w-0.5 rounded-full', status === 'error' ? 'bg-destructive' : 'bg-amber-500')} />;
}

/** Inline chips stay compact inside rows but keep a 24×24 minimum hit area (WCAG 2.5.8; see DESIGN.md). */
const CHIP_CLASS = 'h-6 min-w-6 cursor-pointer font-normal';

/**
 * Clickable `#tag` / `^link` chips plus a document indicator. On the Journals page a chip adds the value to the filters (and
 * goes back to page 1); elsewhere (dashboard "Recent activity") it opens the Journals page filtered by just that value.
 */
export function JournalChips({ data, className }: { data: JournalTransactionItem; className?: string }) {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const setJournalTags = useSetAtom(journalTagsAtom);
  const setJournalLinks = useSetAtom(journalLinksAtom);
  const setJournalKeyword = useSetAtom(journalKeywordAtom);
  const setJournalPage = useSetAtom(journalPageAtom);
  const tags = data.tags ?? [];
  const links = data.links ?? [];
  const documents = hasDocuments(data);
  if (tags.length === 0 && links.length === 0 && !documents) return null;

  const add = (kind: 'tag' | 'link', value: string) => (event: React.MouseEvent) => {
    event.stopPropagation();
    const [setter, other] = kind === 'tag' ? [setJournalTags, setJournalLinks] : [setJournalLinks, setJournalTags];
    setJournalPage(1);
    if (pathname === '/journals') {
      setter((prev) => (prev.includes(value) ? prev : [...prev, value]));
      return;
    }
    setJournalKeyword('');
    setter([value]);
    other([]);
    navigate('/journals');
  };

  return (
    <span className={cn('inline-flex flex-wrap items-center gap-1', className)}>
      {links.map((link) => (
        <Badge
          key={`link-${link}`}
          variant="secondary"
          className={CHIP_CLASS}
          render={<button type="button" />}
          aria-label={t('ledger.journal.filter_by', { value: `^${link}` })}
          onClick={add('link', link)}
        >
          ^{link}
        </Badge>
      ))}
      {tags.map((tag) => (
        <Badge
          key={`tag-${tag}`}
          variant="secondary"
          className={CHIP_CLASS}
          render={<button type="button" />}
          aria-label={t('ledger.journal.filter_by', { value: `#${tag}` })}
          onClick={add('tag', tag)}
        >
          #{tag}
        </Badge>
      ))}
      {documents && <Files className="size-3.5 text-muted-foreground" aria-label={t('ledger.journal.has_documents')} />}
    </span>
  );
}
