import { addYears, endOfDay, endOfMonth, endOfYear, isSameDay, startOfDay, startOfMonth, startOfYear, subMonths, subYears } from 'date-fns';
import { CalendarIcon } from 'lucide-react';
import * as React from 'react';
import type { DateRange } from 'react-day-picker';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Calendar } from '@/components/ui/calendar';
import { Drawer, DrawerContent, DrawerFooter, DrawerHeader, DrawerTitle, DrawerTrigger } from '@/components/ui/drawer';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useIsMobile } from '@/hooks/use-mobile';
import { cn } from '@/lib/utils';
import { formatRange, useDateFormat, useDateLocale } from './use-date-format';

export interface DateRangeValue {
  from: Date;
  to: Date;
}

export interface DateRangePreset {
  key: string;
  label: string;
  range: DateRangeValue;
}

interface Props {
  value: DateRangeValue;
  onChange: (value: DateRangeValue) => void;
  /** Extra shortcuts shown before the built-in ones (e.g. "latest activity"). */
  extraPresets?: DateRangePreset[];
  className?: string;
}

function useBuiltInPresets(): DateRangePreset[] {
  const { t } = useTranslation();
  return React.useMemo(() => {
    const today = new Date();
    const lastMonth = subMonths(today, 1);
    const lastYear = subYears(today, 1);
    return [
      { key: 'this_month', label: t('ledger.range.this_month'), range: { from: startOfMonth(today), to: endOfMonth(today) } },
      { key: 'last_month', label: t('ledger.range.last_month'), range: { from: startOfMonth(lastMonth), to: endOfMonth(lastMonth) } },
      { key: 'last_3_months', label: t('ledger.range.last_3_months'), range: { from: startOfMonth(subMonths(today, 2)), to: endOfMonth(today) } },
      { key: 'last_12_months', label: t('ledger.range.last_12_months'), range: { from: startOfMonth(subMonths(today, 11)), to: endOfMonth(today) } },
      { key: 'this_year', label: t('ledger.range.this_year'), range: { from: startOfYear(today), to: endOfYear(today) } },
      { key: 'last_year', label: t('ledger.range.last_year'), range: { from: startOfYear(lastYear), to: endOfYear(lastYear) } },
    ];
  }, [t]);
}

const sameRange = (a: DateRangeValue, b: DateRangeValue) => isSameDay(a.from, b.from) && isSameDay(a.to, b.to);

/**
 * Date range control: >= md a popover with shortcuts + a two-month calendar; < md a bottom drawer with shortcut chips and a
 * one-month calendar with 40px day cells. Custom ranges are committed with "Apply"; shortcuts apply immediately.
 */
export function DateRangePicker({ value, onChange, extraPresets = [], className }: Props) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const fmt = useDateFormat();
  const locale = useDateLocale();
  const builtIn = useBuiltInPresets();
  const presets = [...extraPresets, ...builtIn];

  const [open, setOpen] = React.useState(false);
  const [draft, setDraft] = React.useState<DateRange | undefined>(value);
  const [month, setMonth] = React.useState<Date>(value.from);

  const onOpenChange = (next: boolean) => {
    if (next) {
      setDraft(value);
      setMonth(isMobile ? value.to : value.from);
    }
    setOpen(next);
  };

  const commit = (range: DateRangeValue) => {
    onChange({ from: startOfDay(range.from), to: endOfDay(range.to) });
    setOpen(false);
  };

  const label = formatRange(fmt, value.from, value.to);
  const draftComplete = !!draft?.from && !!draft?.to;
  const draftLabel = draft?.from ? (draft.to ? formatRange(fmt, draft.from, draft.to) : fmt.date(draft.from)) : t('ledger.range.pick_start');
  const activeKey = presets.find((preset) => sameRange(preset.range, value))?.key;

  const calendar = (
    <Calendar
      mode="range"
      locale={locale}
      selected={draft}
      onSelect={setDraft}
      month={month}
      onMonthChange={setMonth}
      numberOfMonths={isMobile ? 1 : 2}
      captionLayout="dropdown"
      startMonth={new Date(2000, 0)}
      endMonth={endOfYear(addYears(new Date(), 1))}
      className={cn(isMobile && 'mx-auto [--cell-size:--spacing(10)]')}
    />
  );

  const trigger = (
    <Button
      variant="outline"
      className={cn('h-10 w-full justify-start gap-2 font-normal sm:w-auto md:h-8', className)}
      aria-label={`${t('ledger.range.title')}: ${label}`}
    />
  );
  const triggerContent = (
    <>
      <CalendarIcon className="text-muted-foreground" />
      <span className="truncate tabular-nums">{label}</span>
    </>
  );

  if (isMobile) {
    return (
      <Drawer open={open} onOpenChange={onOpenChange}>
        <DrawerTrigger render={trigger}>{triggerContent}</DrawerTrigger>
        <DrawerContent>
          <DrawerHeader>
            <DrawerTitle>{t('ledger.range.title')}</DrawerTitle>
          </DrawerHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-4 pt-4">
            <div className="grid grid-cols-2 gap-2">
              {presets.map((preset) => (
                <Button
                  key={preset.key}
                  variant={preset.key === activeKey ? 'secondary' : 'outline'}
                  className="h-10 justify-start"
                  onClick={() => commit(preset.range)}
                >
                  <span className="truncate">{preset.label}</span>
                </Button>
              ))}
            </div>
            <div className="mt-4 border-t pt-2">{calendar}</div>
          </div>
          <DrawerFooter className="border-t pt-4">
            <p className="text-center text-sm text-muted-foreground tabular-nums">{draftLabel}</p>
            <Button className="h-10" disabled={!draftComplete} onClick={() => draft?.from && draft.to && commit({ from: draft.from, to: draft.to })}>
              {t('ledger.range.apply')}
            </Button>
          </DrawerFooter>
        </DrawerContent>
      </Drawer>
    );
  }

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger render={trigger}>{triggerContent}</PopoverTrigger>
      <PopoverContent className="w-auto p-0" align="end">
        <div className="flex">
          <div className="flex w-40 flex-col gap-0.5 border-r p-2">
            {presets.map((preset) => (
              <Button
                key={preset.key}
                variant={preset.key === activeKey ? 'secondary' : 'ghost'}
                size="sm"
                className="justify-start"
                onClick={() => commit(preset.range)}
              >
                <span className="truncate">{preset.label}</span>
              </Button>
            ))}
          </div>
          <div className="flex flex-col">
            {calendar}
            <div className="flex items-center justify-between gap-3 border-t p-2">
              <span className="px-1 text-xs text-muted-foreground tabular-nums">{draftLabel}</span>
              <div className="flex gap-2">
                <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
                  {t('ledger.common.cancel')}
                </Button>
                <Button size="sm" disabled={!draftComplete} onClick={() => draft?.from && draft.to && commit({ from: draft.from, to: draft.to })}>
                  {t('ledger.range.apply')}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
