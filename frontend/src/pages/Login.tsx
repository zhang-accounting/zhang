import { useSetAtom } from 'jotai';
import { Clock, KeyRound, TriangleAlert } from 'lucide-react';
import { FormEvent, ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { AuthRequestError, login, type AuthStatus } from '@/api/auth';
import { useDefaultPasskeyName, usePasskeyErrorMessage } from '@/components/auth/passkey-hooks';
import { Button } from '@/components/ui/button';
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Spinner } from '@/components/ui/spinner';
import { useDocumentTitle } from '@/hooks/use-document-title';
import { AuthScreen } from '@/layout/AuthScreen';
import { registerPasskey, signInWithPasskey } from '@/lib/passkey';
import { cn } from '@/lib/utils';
import { passkeyUnavailableReason, type PasskeyUnavailableReason } from '@/lib/webauthn';
import { signedInAtom, type LockReason } from '@/states/auth';

const INPUT_CLASS = 'h-10';
const CODE_CLASS = 'rounded-sm bg-muted px-1 py-px font-mono text-[0.8125rem] text-foreground-2';
/** Thrown by the forms when a required field is empty (shown as "enter …", never sent). */
const MISSING_INPUT = new Error('missing input');
const UNAVAILABLE_KEY: Record<PasskeyUnavailableReason, string> = {
  insecure: 'auth.unavailable_insecure',
  unsupported: 'auth.unavailable_unsupported',
  'ip-address': 'auth.unavailable_ip',
};

/** Hint under an action that turns into its error (same slot, so an error does not move the form). */
function StatusLine({ id, error, hint }: { id: string; error?: string; hint?: ReactNode }) {
  return (
    <p id={id} aria-live="polite" className={cn('min-h-5 text-center text-sm text-balance', error ? 'text-destructive' : 'text-muted-foreground')}>
      {error ?? hint}
    </p>
  );
}

/** Guards an async action against double submits and tracks its busy / error state. */
function useAction(action: () => Promise<void>, describe: (error: unknown) => string) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const running = useRef(false);
  const run = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (failure) {
      setError(describe(failure));
    } finally {
      running.current = false;
      setBusy(false);
    }
  }, [action, describe]);
  return { busy, error, setError, run };
}

/** Main action when a passkey is registered: one large button; Enter anywhere outside a control starts it too. */
function PasskeySignIn({ onSignedIn }: { onSignedIn: () => void }) {
  const { t } = useTranslation();
  const statusId = useId();
  const describe = usePasskeyErrorMessage();
  const action = useCallback(async () => {
    await signInWithPasskey();
    onSignedIn();
  }, [onSignedIn]);
  const { busy, error, run } = useAction(
    action,
    useCallback((failure: unknown) => describe(failure, 'sign-in'), [describe]),
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Enter' || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('input, textarea, select, button, a[href], [role], [contenteditable="true"]')) return;
      event.preventDefault();
      void run();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [run]);

  return (
    <div className="flex flex-col gap-2">
      <Button className="h-11 w-full text-[0.9375rem]" onClick={() => void run()} aria-busy={busy} aria-describedby={statusId}>
        {busy ? <Spinner role="presentation" aria-hidden className="size-4" /> : <KeyRound />}
        {busy ? t('auth.passkey_waiting') : t('auth.passkey_button')}
      </Button>
      <StatusLine id={statusId} error={error} hint={t('auth.passkey_hint')} />
    </div>
  );
}

/** First run in passkey mode: the `ZHANG_PASSKEY` secret unlocks the registration of the first passkey (which signs in). */
function PasskeySetup({ onSignedIn }: { onSignedIn: () => void }) {
  const { t } = useTranslation();
  const id = useId();
  const defaultName = useDefaultPasskeyName();
  const [secret, setSecret] = useState('');
  // `null` = untouched: follows the default (which is translated) until the user edits it.
  const [name, setName] = useState<string | null>(null);
  const secretRef = useRef<HTMLInputElement>(null);
  const describe = usePasskeyErrorMessage();
  const action = useCallback(async () => {
    if (!secret) {
      secretRef.current?.focus();
      throw MISSING_INPUT;
    }
    await registerPasskey(secret, (name ?? defaultName).trim() || defaultName);
    onSignedIn();
  }, [secret, name, defaultName, onSignedIn]);
  const { busy, error, run } = useAction(
    action,
    useCallback(
      (failure: unknown) => {
        return failure === MISSING_INPUT ? t('auth.setup_secret_missing') : describe(failure, 'register');
      },
      [describe, t],
    ),
  );
  const secretInvalid = !!error && (!secret || error === t('auth.error_secret'));

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void run();
  };

  return (
    <form className="flex flex-col gap-4" onSubmit={submit} noValidate aria-labelledby={`${id}-title`}>
      <div className="flex flex-col gap-1">
        <h2 id={`${id}-title`} className="text-base font-semibold">
          {t('auth.setup_title')}
        </h2>
        <p className="text-sm text-muted-foreground">{t('auth.setup_description')}</p>
      </div>
      <Field data-invalid={secretInvalid || undefined}>
        <FieldLabel htmlFor={`${id}-secret`}>{t('auth.setup_secret')}</FieldLabel>
        <Input
          ref={secretRef}
          id={`${id}-secret`}
          type="password"
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          enterKeyHint="next"
          value={secret}
          onChange={(event) => setSecret(event.target.value)}
          aria-invalid={secretInvalid || undefined}
          aria-describedby={`${id}-secret-hint`}
          className={INPUT_CLASS}
        />
        <FieldDescription id={`${id}-secret-hint`}>
          <Trans i18nKey="auth.setup_secret_description" components={{ code: <code className={CODE_CLASS} /> }} />
        </FieldDescription>
      </Field>
      <Field>
        <FieldLabel htmlFor={`${id}-name`}>
          {t('auth.setup_name')}
          <span className="font-normal text-muted-foreground">{t('auth.optional')}</span>
        </FieldLabel>
        <Input
          id={`${id}-name`}
          autoComplete="off"
          enterKeyHint="go"
          maxLength={64}
          value={name ?? defaultName}
          onChange={(event) => setName(event.target.value)}
          aria-describedby={`${id}-name-hint`}
          className={INPUT_CLASS}
        />
        <FieldDescription id={`${id}-name-hint`}>{t('auth.setup_name_description')}</FieldDescription>
      </Field>
      <div className="flex flex-col gap-2">
        <Button type="submit" className="h-11 w-full text-[0.9375rem]" aria-busy={busy} aria-describedby={`${id}-status`}>
          {busy ? <Spinner role="presentation" aria-hidden className="size-4" /> : <KeyRound />}
          {busy ? t('auth.setup_creating') : t('auth.setup_submit')}
        </Button>
        <StatusLine id={`${id}-status`} error={error} hint={t('auth.setup_hint')} />
      </div>
    </form>
  );
}

/** Username + password (`ZHANG_AUTH`). Primary only when it is the only way in; outline next to a passkey button. */
function PasswordSignIn({ primary, onSignedIn }: { primary: boolean; onSignedIn: () => void }) {
  const { t } = useTranslation();
  const id = useId();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const usernameRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  const action = useCallback(async () => {
    if (!username.trim() || !password) {
      (username.trim() ? passwordRef : usernameRef).current?.focus();
      throw MISSING_INPUT;
    }
    try {
      await login(username.trim(), password);
    } catch (failure) {
      if (failure instanceof AuthRequestError && failure.status === 401) passwordRef.current?.select();
      throw failure;
    }
    onSignedIn();
  }, [username, password, onSignedIn]);
  const { busy, error, run } = useAction(
    action,
    useCallback(
      (failure: unknown) => {
        if (failure === MISSING_INPUT) return t('auth.password_missing');
        if (failure instanceof AuthRequestError) {
          if (failure.status === 401) return t('auth.wrong_password');
          // 429: too many failed attempts; the server says when to retry.
          if (failure.status === 429) return failure.message;
          if (failure.status === 0) return t('auth.error_network');
        }
        return t('auth.error_generic', { message: failure instanceof Error ? failure.message : String(failure) });
      },
      [t],
    ),
  );
  const invalid = !!error && error === t('auth.wrong_password');

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void run();
  };

  return (
    <form className="flex flex-col gap-4" onSubmit={submit} noValidate aria-label={t('auth.password_form')}>
      <Field data-invalid={invalid || undefined}>
        <FieldLabel htmlFor={`${id}-username`}>{t('auth.username')}</FieldLabel>
        <Input
          ref={usernameRef}
          id={`${id}-username`}
          name="username"
          autoComplete="username"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          enterKeyHint="next"
          value={username}
          onChange={(event) => setUsername(event.target.value)}
          aria-invalid={invalid || undefined}
          className={INPUT_CLASS}
        />
      </Field>
      <Field data-invalid={invalid || undefined}>
        <FieldLabel htmlFor={`${id}-password`}>{t('auth.password')}</FieldLabel>
        <Input
          ref={passwordRef}
          id={`${id}-password`}
          name="password"
          type="password"
          autoComplete="current-password"
          enterKeyHint="go"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          aria-invalid={invalid || undefined}
          aria-describedby={`${id}-status`}
          className={INPUT_CLASS}
        />
      </Field>
      <div className="flex flex-col gap-2">
        <Button
          type="submit"
          variant={primary ? 'default' : 'outline'}
          className={cn('w-full', primary ? 'h-11 text-[0.9375rem]' : 'h-10')}
          aria-busy={busy}
          aria-describedby={`${id}-status`}
        >
          {busy && <Spinner role="presentation" aria-hidden className="size-4" />}
          {busy ? t('auth.signing_in') : t('auth.sign_in')}
        </Button>
        <StatusLine id={`${id}-status`} error={error} hint={<Trans i18nKey="auth.password_hint" components={{ code: <code className={CODE_CLASS} /> }} />} />
      </div>
    </form>
  );
}

function PasskeyUnavailable({ reason, hasPassword }: { reason: PasskeyUnavailableReason; hasPassword: boolean }) {
  const { t } = useTranslation();
  return (
    <div role="note" className="flex gap-2.5 rounded-lg bg-muted px-3 py-2.5 text-sm text-foreground-2">
      <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
      <p className="text-pretty">
        {t(UNAVAILABLE_KEY[reason])} {hasPassword ? t('auth.unavailable_use_password') : t('auth.unavailable_no_fallback')}
      </p>
    </div>
  );
}

function OrDivider() {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground" role="separator" aria-label={t('auth.or')}>
      <span className="h-px flex-1 bg-border" />
      <span aria-hidden>{t('auth.or')}</span>
      <span className="h-px flex-1 bg-border" />
    </div>
  );
}

/**
 * Login page (`/login`, outside the shell): otter + "Sign in to <title>", then the methods the server enabled: the passkey
 * button (or the first-passkey setup form) as the main action, the password form under an "or" divider.
 */
export default function Login({ status, reason }: { status: AuthStatus; reason?: LockReason }) {
  const { t } = useTranslation();
  const signedIn = useSetAtom(signedInAtom);
  // Toasts of the previous session (e.g. requests that failed when it expired) would replay once the shell's <Toaster> mounts.
  const onSignedIn = useCallback(() => {
    toast.dismiss();
    signedIn();
  }, [signedIn]);
  const title = status.title?.trim() || 'Zhang';
  const unavailable = useMemo(() => passkeyUnavailableReason(), []);
  const passkeyOn = status.methods.passkey;
  const passkeyReady = passkeyOn && unavailable === null;
  const passwordOn = status.methods.password;
  const showSignIn = passkeyReady && status.passkey_registered;
  const showSetup = passkeyReady && !status.passkey_registered;

  useDocumentTitle(`${t('auth.sign_in')} - ${title}`);

  const subtitle = showSignIn ? t('auth.subtitle_passkey') : showSetup ? undefined : passwordOn ? t('auth.subtitle_password') : undefined;

  return (
    <AuthScreen>
      <div className="flex w-full max-w-sm flex-col gap-6">
        <header className="flex flex-col items-center gap-3 text-center">
          <img src="/otter-192.png" alt="" width={48} height={48} className="size-12 rounded-lg" />
          <div className="flex min-w-0 flex-col gap-1.5">
            <h1 className="text-xl font-semibold tracking-tight text-balance break-words">{t('auth.sign_in_to', { title })}</h1>
            {subtitle && <p className="text-sm text-balance text-muted-foreground">{subtitle}</p>}
          </div>
        </header>

        {reason === 'expired' && (
          <p role="status" className="flex items-center gap-2 rounded-lg border bg-card px-3 py-2 text-sm text-foreground-2">
            <Clock className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            {t('auth.session_expired')}
          </p>
        )}

        <div className="flex flex-col gap-4 rounded-xl border bg-card p-5 pb-4 shadow-xs sm:p-6 sm:pb-5">
          {passkeyOn && unavailable && <PasskeyUnavailable reason={unavailable} hasPassword={passwordOn} />}
          {showSignIn && <PasskeySignIn onSignedIn={onSignedIn} />}
          {showSetup && <PasskeySetup onSignedIn={onSignedIn} />}
          {passkeyReady && passwordOn && <OrDivider />}
          {passwordOn && <PasswordSignIn primary={!passkeyReady} onSignedIn={onSignedIn} />}
          {!passkeyOn && !passwordOn && <p className="pb-2 text-sm text-muted-foreground">{t('auth.no_methods')}</p>}
        </div>
      </div>
    </AuthScreen>
  );
}
