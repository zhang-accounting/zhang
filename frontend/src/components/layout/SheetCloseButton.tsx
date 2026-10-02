import { X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { SheetClose } from '@/components/ui/sheet';

/**
 * Close button for `<SheetContent showCloseButton={false}>`: the CLI sheet's built-in one is 28px with an untranslated
 * "Close" label; this one is 40px on mobile (DESIGN.md tap targets) and labelled in the UI language.
 */
export function SheetCloseButton() {
  const { t } = useTranslation();
  return (
    <SheetClose render={<Button variant="ghost" size="icon" className="absolute top-2 right-2 size-10 md:size-8" aria-label={t('ledger.common.close')} />}>
      <X />
    </SheetClose>
  );
}
