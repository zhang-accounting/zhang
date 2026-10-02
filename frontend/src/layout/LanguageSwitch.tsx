import { Languages } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { DropdownMenuRadioGroup, DropdownMenuRadioItem } from '@/components/ui/dropdown-menu';
import { useLanguage } from '@/hooks/use-language';
import { LANGUAGES } from '@/lib/languages';

/** Icon dropdown (desktop top bar) to switch the UI language. */
export function LanguageSwitch() {
  const { t } = useTranslation();
  const [lang, setLang] = useLanguage();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={t('SHELL_LANGUAGE')} />}>
        <Languages />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-36">
        <DropdownMenuRadioGroup value={lang} onValueChange={(value) => setLang(String(value))}>
          {LANGUAGES.map((language) => (
            <DropdownMenuRadioItem key={language.value} value={language.value}>
              {language.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
