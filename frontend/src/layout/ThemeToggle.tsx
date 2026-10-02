import { Moon, Sun } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { THEMES } from './themes';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { DropdownMenuRadioGroup, DropdownMenuRadioItem } from '@/components/ui/dropdown-menu';

/** Icon dropdown (desktop top bar) to pick light / dark / system theme via next-themes. */
export function ThemeToggle() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={t('SHELL_THEME')} />}>
        <Sun className="scale-100 rotate-0 transition-[scale,rotate] motion-reduce:transition-none dark:scale-0 dark:-rotate-90" />
        <Moon className="absolute scale-0 rotate-90 transition-[scale,rotate] motion-reduce:transition-none dark:scale-100 dark:rotate-0" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuRadioGroup value={theme ?? 'system'} onValueChange={(value) => setTheme(String(value))}>
          {THEMES.map((item) => (
            <DropdownMenuRadioItem key={item.value} value={item.value}>
              <item.icon />
              {t(item.label)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
