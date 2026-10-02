import { format as formatDate } from 'date-fns';
import { enUS, zhCN } from 'date-fns/locale';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

/** date-fns locale matching the active i18next language. */
export function useDateLocale() {
  const { i18n } = useTranslation();
  return i18n.language?.startsWith('zh') ? zhCN : enUS;
}

/** Locale-aware date formatters shared by the ledger pages (English month names vs `2023年9月16日`). */
export function useDateFormat() {
  const locale = useDateLocale();
  return useMemo(() => {
    const zh = locale === zhCN;
    const format = (date: Date | number, pattern: string) => formatDate(date, pattern, { locale });
    return {
      locale,
      format,
      /** `Sep 16` / `9月16日` */
      day: (date: Date | number) => format(date, zh ? 'M月d日' : 'MMM d'),
      /** `Sep 16, 2023` / `2023年9月16日` */
      date: (date: Date | number) => format(date, 'PP'),
      /** `Sat, Sep 16, 2023` / `2023年9月16日 周六` */
      weekdayDate: (date: Date | number) => format(date, zh ? 'PP EEE' : 'EEE, PP'),
      /** `Sep 2023` / `2023年9月` */
      month: (date: Date | number) => format(date, zh ? 'yyyy年M月' : 'MMM yyyy'),
      /** `Sep 16 21:56` / `9月16日 21:56` */
      dayTime: (date: Date | number) => format(date, zh ? 'M月d日 HH:mm' : 'MMM d HH:mm'),
      /** `Sep 16, 2023 21:56` */
      dateTime: (date: Date | number) => format(date, 'PP HH:mm'),
      /** `21:56` */
      time: (date: Date | number) => format(date, 'HH:mm'),
    };
  }, [locale]);
}

/** `true` when the dates fall in more than one calendar year (time axes then need the year in their tick labels). */
export function spansYears(dates: (Date | number)[]) {
  return new Set(dates.map((date) => new Date(date).getFullYear())).size > 1;
}

/** `from – to` label for a date range, collapsing the year when both ends share it. */
export function formatRange(fmt: ReturnType<typeof useDateFormat>, from: Date, to: Date) {
  if (from.getFullYear() === to.getFullYear() && from.getFullYear() === new Date().getFullYear()) {
    return `${fmt.day(from)} – ${fmt.day(to)}`;
  }
  return `${fmt.date(from)} – ${fmt.date(to)}`;
}
