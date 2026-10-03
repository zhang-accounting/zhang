import { useAtomValue, useSetAtom } from 'jotai';
import { KeyRound, Plus, Trash2 } from 'lucide-react';
import { FormEvent, useCallback, useEffect, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { AuthRequestError, deletePasskey, listPasskeys, type PasskeyInfo } from '@/api/auth';
import { SettingsSection } from '@/components/basic/Setting';
import { EmptyState } from '@/components/layout';
import { useDateFormat } from '@/components/layout/use-date-format';
import { AutoDrawer, AutoDrawerTrigger } from '@/components/ui/auto-drawer';
import { Button } from '@/components/ui/button';
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Skeleton } from '@/components/ui/skeleton';
import { Spinner } from '@/components/ui/spinner';
import { apiErrorMessage } from '@/lib/api-error';
import { registerPasskey } from '@/lib/passkey';
import { passkeyUnavailableReason } from '@/lib/webauthn';
import { loadAuthStatusAtom, passkeyModeAtom } from '@/states/auth';
import { useDefaultPasskeyName, usePasskeyErrorMessage } from './passkey-hooks';

const UNAVAILABLE_KEY = {
  insecure: 'auth.unavailable_insecure',
  unsupported: 'auth.unavailable_unsupported',
  'ip-address': 'auth.unavailable_ip',
} as const;

/** `created_at` as RFC 3339 or a unix timestamp in seconds / milliseconds. */
function parseCreatedAt(value: PasskeyInfo['created_at']): Date | undefined {
  const date = typeof value === 'number' ? new Date(value < 1e12 ? value * 1000 : value) : new Date(value);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/** "Add passkey" (signed in, so no registration secret): name it, then the browser creates it. */
function AddPasskeyDialog({ disabled, onAdded }: { disabled: boolean; onAdded: () => void }) {
  const { t } = useTranslation();
  const id = useId();
  const defaultName = useDefaultPasskeyName();
  const describe = usePasskeyErrorMessage();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const onOpenChange = (next: boolean) => {
    if (busy) return;
    setOpen(next);
    if (next) {
      setName(null);
      setError(undefined);
    }
  };

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(undefined);
    try {
      await registerPasskey(null, (name ?? defaultName).trim() || defaultName);
      toast.success(t('settings.passkeys_added'));
      setOpen(false);
      onAdded();
    } catch (failure) {
      setError(describe(failure, 'register'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <AutoDrawer
      open={open}
      onOpenChange={onOpenChange}
      title={t('settings.passkeys_add_title')}
      description={t('settings.passkeys_add_description')}
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => onOpenChange(false)} disabled={busy}>
            {t('settings.passkeys_cancel')}
          </Button>
          <Button className="h-10 md:h-8" onClick={() => void submit()} aria-busy={busy}>
            {busy ? <Spinner role="presentation" aria-hidden /> : <KeyRound />}
            {busy ? t('auth.setup_creating') : t('auth.setup_submit')}
          </Button>
        </>
      }
    >
      <AutoDrawerTrigger render={<Button variant="outline" className="h-10 md:h-8" disabled={disabled} />}>
        <Plus />
        {t('settings.passkeys_add')}
      </AutoDrawerTrigger>
      <form className="flex flex-col gap-3 pb-1" onSubmit={submit} noValidate>
        <Field>
          <FieldLabel htmlFor={`${id}-name`}>{t('auth.setup_name')}</FieldLabel>
          <Input
            id={`${id}-name`}
            autoComplete="off"
            enterKeyHint="go"
            maxLength={64}
            value={name ?? defaultName}
            onChange={(event) => setName(event.target.value)}
            aria-describedby={`${id}-hint`}
            className="h-10 md:h-8"
          />
          <FieldDescription id={`${id}-hint`}>{t('auth.setup_name_description')}</FieldDescription>
        </Field>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </form>
    </AutoDrawer>
  );
}

/** Confirmation before revoking a passkey; the server refuses (409) to remove the last one when password sign-in is off. */
function RemovePasskeyDialog({ passkey, onRemoved }: { passkey: PasskeyInfo; onRemoved: () => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const onOpenChange = (next: boolean) => {
    if (busy) return;
    setOpen(next);
    if (next) setError(undefined);
  };

  const remove = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await deletePasskey(passkey.id);
      toast.success(t('settings.passkeys_removed'));
      setOpen(false);
      onRemoved();
    } catch (failure) {
      const message = await apiErrorMessage(failure);
      setError(failure instanceof AuthRequestError && failure.status === 409 ? message : t('settings.passkeys_remove_failed', { message }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <AutoDrawer
      open={open}
      onOpenChange={onOpenChange}
      title={t('settings.passkeys_remove_title')}
      description={t('settings.passkeys_remove_description', { name: passkey.name })}
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => onOpenChange(false)} disabled={busy}>
            {t('settings.passkeys_cancel')}
          </Button>
          <Button variant="destructive" className="h-10 md:h-8" onClick={() => void remove()} aria-busy={busy}>
            {busy ? <Spinner role="presentation" aria-hidden /> : <Trash2 />}
            {t('settings.passkeys_remove')}
          </Button>
        </>
      }
    >
      <AutoDrawerTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="size-10 shrink-0 text-muted-foreground hover:text-destructive md:size-8"
            aria-label={t('settings.passkeys_remove_label', { name: passkey.name })}
            title={t('settings.passkeys_remove')}
          />
        }
      >
        <Trash2 />
      </AutoDrawerTrigger>
      {error && (
        <p role="alert" className="pb-1 text-sm text-destructive">
          {error}
        </p>
      )}
    </AutoDrawer>
  );
}

function PasskeyRow({ passkey, onRemoved }: { passkey: PasskeyInfo; onRemoved: () => void }) {
  const { t } = useTranslation();
  const fmt = useDateFormat();
  const created = parseCreatedAt(passkey.created_at);
  return (
    <li className="flex items-center gap-3 py-2.5 pr-2 pl-4">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground" aria-hidden>
        <KeyRound className="size-4" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium" title={passkey.name}>
          {passkey.name}
        </span>
        <span className="text-xs text-muted-foreground">
          {t('settings.passkeys_created', { date: created ? fmt.date(created) : String(passkey.created_at) })}
        </span>
      </div>
      <RemovePasskeyDialog passkey={passkey} onRemoved={onRemoved} />
    </li>
  );
}

/** Settings → Passkeys (passkey mode only): registered passkeys, add one on this device, remove with confirmation. */
export function PasskeySettings() {
  const { t } = useTranslation();
  const passkeyMode = useAtomValue(passkeyModeAtom);
  const loadAuthStatus = useSetAtom(loadAuthStatusAtom);
  const unavailable = useMemo(() => passkeyUnavailableReason(), []);
  const [passkeys, setPasskeys] = useState<PasskeyInfo[]>();
  const [loadError, setLoadError] = useState<string>();

  const load = useCallback(async () => {
    try {
      setPasskeys(await listPasskeys());
      setLoadError(undefined);
    } catch (error) {
      setLoadError(await apiErrorMessage(error));
    }
  }, []);

  useEffect(() => {
    if (passkeyMode) void load();
  }, [passkeyMode, load]);

  const changed = useCallback(() => {
    void load();
    void loadAuthStatus({ quiet: true });
  }, [load, loadAuthStatus]);

  if (!passkeyMode) return null;

  return (
    <SettingsSection
      title={t('settings.passkeys')}
      description={unavailable ? t(UNAVAILABLE_KEY[unavailable]) : t('settings.passkeys_description')}
      action={<AddPasskeyDialog disabled={!!unavailable} onAdded={changed} />}
      bare={!!passkeys && passkeys.length === 0}
    >
      {loadError ? (
        <p role="alert" className="p-4 text-sm text-destructive">
          {t('settings.passkeys_load_failed', { message: loadError })}
        </p>
      ) : !passkeys ? (
        <div className="flex items-center gap-3 p-4">
          <Skeleton className="size-8 rounded-md" />
          <Skeleton className="h-4 w-40" />
        </div>
      ) : passkeys.length === 0 ? (
        <EmptyState icon={KeyRound} title={t('settings.passkeys_empty_title')} description={t('settings.passkeys_empty_description')} className="py-8" />
      ) : (
        <ul className="divide-y">
          {passkeys.map((passkey) => (
            <PasskeyRow key={passkey.id} passkey={passkey} onRemoved={changed} />
          ))}
        </ul>
      )}
    </SettingsSection>
  );
}
