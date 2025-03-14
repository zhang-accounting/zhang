import { Search } from 'lucide-react';

import NewTransactionButton from '@/components/NewTransactionButton';
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbSeparator } from '@/components/ui/breadcrumb';
import { Input } from '@/components/ui/input';
import { breadcrumbAtom } from '@/states/basic';
import { useAtomValue } from 'jotai';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';

export function Nav() {
  const { t } = useTranslation();
  const breadcrumb = useAtomValue(breadcrumbAtom);
  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-4 border-b bg-background px-4 sm:static sm:h-auto sm:border-0 sm:bg-transparent sm:px-6">
      <Breadcrumb className="hidden md:flex">
        <BreadcrumbList>
          {breadcrumb.map((item, index) => (
            <BreadcrumbItem key={item.uri}>
              <BreadcrumbLink asChild>
                <Link to={item.uri}>{(item.noTranslate ?? false) ? item.label : t(item.label)}</Link>
              </BreadcrumbLink>
              {index < breadcrumb.length - 1 && <BreadcrumbSeparator />}
            </BreadcrumbItem>
          ))}
        </BreadcrumbList>
      </Breadcrumb>

      <div className="flex-1 flex items-center gap-4 md:justify-end">
        <div className="relative ml-auto flex-1 md:grow-0 items-center flex gap-4">
          <NewTransactionButton />
          <div className="relative">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input type="search" placeholder="Search..." className="w-full rounded-lg bg-background pl-8 md:w-[200px] lg:w-[320px]" />
          </div>
        </div>
      </div>
    </header>
  );
}
