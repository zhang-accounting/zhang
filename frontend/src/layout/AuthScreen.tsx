import { ReactNode } from 'react';
import { cn } from '@/lib/utils';
import { LanguageSwitch } from './LanguageSwitch';
import { ThemeToggle } from './ThemeToggle';

const FOOTER_BUTTON_CLASS = 'size-10 text-muted-foreground hover:bg-accent hover:text-foreground md:size-8';

/** Full-screen frame without the app shell (login page, auth loading / error): centred content, theme + language below. */
export function AuthScreen({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className="flex min-h-svh flex-col bg-background">
      <main className={cn('flex flex-1 flex-col items-center justify-center px-4 pt-[max(2.5rem,env(safe-area-inset-top))] pb-6 sm:pb-10', className)}>
        {children}
      </main>
      <footer className="flex items-center justify-center gap-1 pb-[max(1rem,env(safe-area-inset-bottom))]">
        <ThemeToggle className={FOOTER_BUTTON_CLASS} side="top" />
        <LanguageSwitch className={FOOTER_BUTTON_CLASS} side="top" />
      </footer>
    </div>
  );
}
