import { ReactNode } from 'react';
import NetworkStatus from '@/components/NetworkStatus';
import { SidebarInset, SidebarProvider } from '@/components/ui/sidebar';
import { AppSidebar } from './AppSidebar';
import { MobileTabBar } from './MobileTabBar';
import { TopBar } from './TopBar';

function sidebarDefaultOpen() {
  // shadcn's SidebarProvider persists its state in the `sidebar_state` cookie.
  return !document.cookie.split('; ').includes('sidebar_state=false');
}

/**
 * Application frame. >= md: collapsible sidebar + sticky top bar. < md: top bar + bottom tab bar with a "More" sheet.
 * Children (routes) render inside a centred max-w-7xl column; bottom padding clears the mobile tab bar.
 */
export function AppShell({ children }: { children: ReactNode }) {
  return (
    <SidebarProvider defaultOpen={sidebarDefaultOpen()}>
      <AppSidebar />
      <SidebarInset className="min-w-0">
        <TopBar />
        <div className="flex-1 overflow-x-clip">
          <div className="mx-auto w-full max-w-7xl px-4 pt-4 pb-[calc(5rem+env(safe-area-inset-bottom))] md:px-6 md:py-6">{children}</div>
        </div>
      </SidebarInset>
      <MobileTabBar />
      <NetworkStatus />
    </SidebarProvider>
  );
}
