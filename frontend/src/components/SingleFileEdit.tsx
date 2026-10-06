import CodeMirror, { EditorView } from '@uiw/react-codemirror';
import { Check, RefreshCw, RotateCcw, Save, TriangleAlert } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useCallback, useEffect, useMemo, useReducer, useState } from 'react';
import { useTranslation } from 'react-i18next';
// eslint-disable-next-line no-restricted-imports -- the file content must not follow ledger reloads: one would overwrite unsaved text
import { useAsync } from 'react-use';
import { toast } from 'sonner';
import { base64Path, retrieveFile, updateFile } from '@/api/requests';
import { EmptyState } from '@/components/layout';
import { editorReducer, initialEditorState, isConflict, isDirty } from '@/components/single-file-edit-state';
import { useUnsavedChangesGuard } from '@/hooks/use-unsaved-changes-guard';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { Button } from './ui/button';
import { Kbd } from './ui/kbd';
import { Skeleton } from './ui/skeleton';
import { Spinner } from './ui/spinner';

interface Props {
  path: string;
  /** The line (1-based) to put the cursor on, scrolled into view, once the file is loaded; past the end of the file, its last line. */
  line?: number | null;
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

/** Puts the cursor at the start of `line` (or of the last line, when the file is shorter) and scrolls it to the middle of the editor. */
function revealLine(view: EditorView, line: number) {
  const { from } = view.state.doc.line(Math.min(Math.max(line, 1), view.state.doc.lines));
  view.dispatch({ selection: { anchor: from }, effects: EditorView.scrollIntoView(from, { y: 'center' }) });
  view.focus();
}

/**
 * CodeMirror editor for one ledger file, filling its container, with a save bar (Ctrl/Cmd+S) at the bottom.
 *
 * A save carries the fingerprint of the file as loaded, so the server refuses to overwrite a file that changed since (a
 * transaction recorded in the app, an uploaded document, an edit outside). The editor then offers to reload the file,
 * discarding the buffer, or to keep editing; it never overwrites the change silently.
 *
 * `line` opens the file at a line (the error list opens the file of an error at its directive, #493).
 */
export default function SingleFileEdit({ path, line, onDirtyChange, className }: Props) {
  const { t } = useTranslation();
  const { resolvedTheme } = useTheme();
  const encodedPath = useMemo(() => base64Path(path), [path]);
  const [state, dispatch] = useReducer(editorReducer, initialEditorState);
  const { content, saved, sha256, saving, conflict } = state;
  // bumped to load the file again on the user's request, after a refused save
  const [reloads, setReloads] = useState(0);

  const { error, loading } = useAsync(async () => {
    const response = await retrieveFile({ file_path: encodedPath });
    dispatch({ type: 'loaded', content: response.data.data.content, sha256: response.data.data.sha256 });
    return response.data.data;
  }, [encodedPath, reloads]);

  const dirty = isDirty(state);
  useEffect(() => onDirtyChange?.(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  useUnsavedChangesGuard(dirty, t('raw_edit.leave_confirm'));

  const onUpdate = useCallback(async () => {
    if (!dirty || saving || sha256 === null) return;
    dispatch({ type: 'save_started' });
    try {
      await updateFile({ file_path: encodedPath, content, expected_sha256: sha256 });
      // the save answers with no body: the file is read back for the fingerprint the next save needs
      const written = await retrieveFile({ file_path: encodedPath })
        .then((response) => response.data.data)
        .catch(() => null);
      dispatch({ type: 'saved', saved: written?.content ?? content, sha256: written?.sha256 ?? null });
      toast.success(t('raw_edit.saved_toast'), { description: t('raw_edit.saved_toast_description') });
    } catch (e) {
      if (isConflict(e)) {
        dispatch({ type: 'save_refused' });
      } else {
        dispatch({ type: 'save_failed' });
        toast.error(t('raw_edit.save_failed'), { description: await apiErrorMessage(e) });
      }
    }
  }, [content, dirty, encodedPath, saving, sha256, t]);

  const reloadFile = useCallback(() => {
    if (!window.confirm(t('raw_edit.conflict_reload_confirm'))) return;
    setReloads((count) => count + 1);
  }, [t]);

  const extensions = useMemo(() => [EditorView.lineWrapping, EDITOR_THEME], []);
  const lineCount = useMemo(() => content.split('\n').length, [content]);

  // the editor is created once the file is loaded, and again after a reload: the line is revealed in each (an editor
  // whose DOM is gone was destroyed by the reload; the next one reveals the line itself)
  const [view, setView] = useState<EditorView | null>(null);
  useEffect(() => {
    if (view !== null && line != null && view.dom.isConnected) revealLine(view, line);
  }, [view, line]);

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
            onChange={(value) => dispatch({ type: 'edited', content: value })}
            onCreateEditor={setView}
            aria-label={path}
          />
        )}
      </div>
      {conflict && (
        <div role="alert" className="flex shrink-0 flex-col gap-2 border-t border-warning/40 bg-warning/10 px-3 py-2 text-xs sm:flex-row sm:items-center">
          <TriangleAlert className="hidden size-4 shrink-0 text-warning sm:block" aria-hidden />
          <div className="min-w-0 flex-1">
            <p className="font-medium text-foreground">{t('raw_edit.conflict_title')}</p>
            <p className="text-muted-foreground">{t('raw_edit.conflict_description')}</p>
          </div>
          <div className="flex shrink-0 gap-2">
            <Button variant="outline" size="sm" onClick={() => dispatch({ type: 'keep_editing' })}>
              {t('raw_edit.conflict_keep_editing')}
            </Button>
            <Button size="sm" onClick={reloadFile}>
              <RefreshCw />
              {t('raw_edit.conflict_reload')}
            </Button>
          </div>
        </div>
      )}
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
          onClick={() => saved !== null && dispatch({ type: 'edited', content: saved })}
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
