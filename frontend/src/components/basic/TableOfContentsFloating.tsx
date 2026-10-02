import { ChevronDown, ChevronRight, FileText, Folder, FolderOpen } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { SheetCloseButton } from '../layout/SheetCloseButton';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '../ui/sheet';
import { cn } from '@/lib/utils';
import { Tier, ZHANG_VALUE } from './file-tree';

interface FileTreeProps {
  files: Tier;
  selected?: string | null;
  /** Path with unsaved changes (shows a dot). */
  dirtyPath?: string | null;
  onChange(value: string): void;
  className?: string;
}

const ROW_CLASS = cn(
  'flex h-10 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left text-sm transition-colors outline-none md:h-8',
  'hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50',
);

function sortedKeys(tier: Tier) {
  // folders first, then files, both alphabetical
  return Object.keys(tier).sort((a, b) => {
    const aFile = !!tier[a][ZHANG_VALUE];
    const bFile = !!tier[b][ZHANG_VALUE];
    if (aFile !== bFile) return aFile ? 1 : -1;
    return a.localeCompare(b);
  });
}

function FolderNode({ name, tier, depth, ...props }: Omit<FileTreeProps, 'files' | 'className'> & { name: string; tier: Tier; depth: number }) {
  const [open, setOpen] = useState(true);
  return (
    <li>
      <button type="button" className={ROW_CLASS} style={{ paddingLeft: 8 + depth * 14 }} aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" /> : <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />}
        {open ? <FolderOpen className="size-4 shrink-0 text-muted-foreground" /> : <Folder className="size-4 shrink-0 text-muted-foreground" />}
        <span className="truncate">{name}</span>
      </button>
      {open && <TreeLevel tier={tier} depth={depth + 1} {...props} />}
    </li>
  );
}

function TreeLevel({ tier, depth, selected, dirtyPath, onChange }: Omit<FileTreeProps, 'files' | 'className'> & { tier: Tier; depth: number }) {
  return (
    <ul className="flex flex-col gap-0.5">
      {sortedKeys(tier).map((key) => {
        const node = tier[key];
        const path = node[ZHANG_VALUE];
        if (!path) return <FolderNode key={key} name={key} tier={node} depth={depth} selected={selected} dirtyPath={dirtyPath} onChange={onChange} />;
        const active = path === selected;
        return (
          <li key={key}>
            <button
              type="button"
              className={cn(ROW_CLASS, active && 'bg-muted font-medium text-foreground')}
              style={{ paddingLeft: 8 + depth * 14 + (depth > 0 ? 18 : 0) }}
              aria-current={active ? 'true' : undefined}
              title={path}
              onClick={() => onChange(path)}
            >
              <FileText className={cn('size-4 shrink-0', active ? 'text-link' : 'text-muted-foreground')} />
              <span className="truncate">{key}</span>
              {dirtyPath === path && <span className="ml-auto size-2 shrink-0 rounded-full bg-warning" aria-hidden />}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** Folder / file tree of the ledger sources (desktop sidebar and mobile sheet). */
export function FileTree({ files, className, ...props }: FileTreeProps) {
  return (
    <nav className={className}>
      <TreeLevel tier={files} depth={0} {...props} />
    </nav>
  );
}

/** Mobile file picker: a full-width trigger showing the current file that opens the tree in a bottom sheet. */
export function TableOfContentsFloating({ files, selected, dirtyPath, onChange, className }: FileTreeProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger render={<Button variant="outline" className={cn('h-10 min-w-0 justify-start gap-2 px-3', className)} />}>
        <FileText className="text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-left">{selected ?? t('raw_edit.choose_file')}</span>
        {dirtyPath && dirtyPath === selected && <span className="size-2 shrink-0 rounded-full bg-warning" aria-label={t('raw_edit.unsaved')} />}
        <ChevronDown className="text-muted-foreground" />
      </SheetTrigger>
      <SheetContent side="bottom" showCloseButton={false} className="max-h-[80svh] gap-0 rounded-t-2xl pb-[env(safe-area-inset-bottom)]">
        <SheetHeader className="pr-12">
          <SheetTitle>{t('raw_edit.files')}</SheetTitle>
          <SheetDescription>{t('raw_edit.files_description')}</SheetDescription>
        </SheetHeader>
        <FileTree
          files={files}
          selected={selected}
          dirtyPath={dirtyPath}
          className="min-h-0 overflow-y-auto overscroll-contain px-2 pb-4"
          onChange={(value) => {
            setOpen(false);
            onChange(value);
          }}
        />
        <SheetCloseButton />
      </SheetContent>
    </Sheet>
  );
}
