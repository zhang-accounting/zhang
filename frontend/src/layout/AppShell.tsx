import { ReactNode } from 'react';
import NetworkStatus from '@/components/NetworkStatus';
import { SidebarInset, SidebarProvider } from '@/components/ui/sidebar';
import { AppSidebar } from './AppSidebar';
import { MobileTabBar } from './MobileTabBar';
import { ReloadFailureNotice } from './ReloadFailureNotice';
import { TopBar } from './TopBar';

function sidebarDefaultOpen() {
  // shadcn's SidebarProvider persists its state in the `sidebar_state` cookie.
  return !document.cookie.split('; ').includes('sidebar_state=false');
}

/**
 * Application frame. >= md: collapsible sidebar, no top bar (pages show their breadcrumb trail in `PageHeader`). < md: top bar
 * (otter / back + title + "+") and a bottom tab bar with a "More" sheet. Routes render inside a centred max-w-7xl column; bottom
 * padding clears the mobile tab bar.
 */
export function AppShell({ children }: { children: ReactNode }) {
  return (
    <SidebarProvider defaultOpen={sidebarDefaultOpen()}>
      <AppSidebar />
      <SidebarInset className="min-w-0">
        <TopBar />
        <ReloadFailureNotice />
        <div className="flex-1 overflow-x-clip">
          <div className="mx-auto w-full max-w-7xl px-4 pt-4 pb-[calc(5rem+env(safe-area-inset-bottom))] md:px-7 md:pt-5 md:pb-10">{children}</div>
        </div>
      </SidebarInset>
      <MobileTabBar />
      <NetworkStatus />
    </SidebarProvider>
  );
}
