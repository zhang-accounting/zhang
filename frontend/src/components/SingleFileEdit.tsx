import CodeMirror, { EditorView } from '@uiw/react-codemirror';
import { Check, RotateCcw, Save, TriangleAlert } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAsync } from 'react-use';
import { toast } from 'sonner';
import { base64Path, retrieveFile, updateFile } from '@/api/requests';
import { EmptyState } from '@/components/layout';
import { useUnsavedChangesGuard } from '@/hooks/use-unsaved-changes-guard';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { Button } from './ui/button';
import { Kbd } from './ui/kbd';
import { Skeleton } from './ui/skeleton';
import { Spinner } from './ui/spinner';

interface Props {
  path: string;
  /** Called whenever the buffer starts / stops differing from the saved file. */
  onDirtyChange?: (dirty: boolean) => void;
  className?: string;
}

const EDITOR_THEME = EditorView.theme({
  '&': { height: '100%', fontSize: '13px' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-gutters': { backgroundColor: 'transparent', borderRight: '1px solid var(--border)' },
  '&.cm-focused': { outline: 'none' },
});

/** CodeMirror editor for one ledger file, filling its container, with a save bar (Ctrl/Cmd+S) at the bottom. */
export default function SingleFileEdit({ path, onDirtyChange, className }: Props) {
  const { t } = useTranslation();
  const { resolvedTheme } = useTheme();
  const encodedPath = useMemo(() => base64Path(path), [path]);
  const [content, setContent] = useState('');
  const [saved, setSaved] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const { error, loading } = useAsync(async () => {
    const response = await retrieveFile({ file_path: encodedPath });
    setContent(response.data.data.content);
    setSaved(response.data.data.content);
    return response.data.data;
  }, [encodedPath]);

  const dirty = saved !== null && content !== saved;
  useEffect(() => onDirtyChange?.(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  useUnsavedChangesGuard(dirty, t('raw_edit.leave_confirm'));

  const onUpdate = useCallback(async () => {
    if (!dirty || saving) return;
    setSaving(true);
    try {
      await updateFile({ file_path: encodedPath, content });
      setSaved(content);
      toast.success(t('raw_edit.saved_toast'), { description: t('raw_edit.saved_toast_description') });
    } catch (e) {
      toast.error(t('raw_edit.save_failed'), { description: await apiErrorMessage(e) });
    } finally {
      setSaving(false);
    }
  }, [content, dirty, encodedPath, saving, t]);

  const extensions = useMemo(() => [EditorView.lineWrapping, EDITOR_THEME], []);
  const lineCount = useMemo(() => content.split('\n').length, [content]);

  if (error) {
    return <EmptyState icon={TriangleAlert} title={t('raw_edit.load_failed')} description={error.message} className="m-4 flex-1" />;
  }

  return (
    <div
      className={cn('flex min-h-0 flex-col', className)}
      onKeyDown={(event) => {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
          event.preventDefault();
          onUpdate();
        }
      }}
    >
      <div className="min-h-0 flex-1 overflow-hidden">
        {loading || saved === null ? (
          <div className="flex flex-col gap-2 p-4">
            {Array.from({ length: 8 }, (_, index) => (
              <Skeleton key={index} className="h-4" style={{ width: `${40 + ((index * 37) % 50)}%` }} />
            ))}
          </div>
        ) : (
          <CodeMirror
            value={content}
            height="100%"
            width="100%"
            className="h-full [&_.cm-editor]:bg-card! [&_.cm-gutters]:bg-card! [&_.cm-activeLine]:bg-muted/40! [&_.cm-activeLineGutter]:bg-muted!"
            theme={resolvedTheme === 'dark' ? 'dark' : 'light'}
            extensions={extensions}
            onChange={(value) => setContent(value)}
            aria-label={path}
          />
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2 border-t bg-muted/40 px-3 py-2">
        <div className="flex min-w-0 flex-1 items-center gap-2 text-xs text-muted-foreground" aria-live="polite">
          {dirty ? (
            <>
              <span className="size-2 shrink-0 rounded-full bg-warning" aria-hidden />
              <span className="truncate font-medium text-foreground">{t('raw_edit.unsaved')}</span>
            </>
          ) : (
            <>
              <Check className="size-3.5 shrink-0" />
              <span className="truncate">{t('raw_edit.all_saved')}</span>
            </>
          )}
          <span className="hidden tabular-nums sm:inline">· {t('raw_edit.line_count', { count: lineCount })}</span>
        </div>
        <Button
          variant="ghost"
          className="h-10 md:h-8"
          aria-label={t('raw_edit.discard')}
          disabled={!dirty || saving}
          onClick={() => saved !== null && setContent(saved)}
        >
          <RotateCcw />
          <span className="hidden sm:inline">{t('raw_edit.discard')}</span>
        </Button>
        <Button className="h-10 md:h-8" disabled={!dirty || saving} onClick={onUpdate}>
          {saving ? <Spinner /> : <Save />}
          {t('SAVE')}
          <Kbd className="hidden bg-primary-foreground/15 text-primary-foreground md:inline-flex">{navigator.platform.includes('Mac') ? '⌘S' : 'Ctrl S'}</Kbd>
        </Button>
      </div>
    </div>
  );
}
