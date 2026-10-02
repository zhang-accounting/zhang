import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { PasskeyError } from '@/lib/passkey';
import { describeDevice } from '@/lib/webauthn';

/** One human sentence for a failed passkey sign-in / registration (`PasskeyError` kinds, see lib/passkey). */
export function usePasskeyErrorMessage() {
  const { t } = useTranslation();
  return useCallback(
    (error: unknown, action: 'sign-in' | 'register') => {
      const message = error instanceof Error ? error.message : String(error);
      if (!(error instanceof PasskeyError)) return t('auth.error_generic', { message });
      switch (error.kind) {
        case 'dismissed':
          return t('auth.error_dismissed');
        case 'exists':
          return t('auth.error_exists');
        case 'security':
          return t('auth.error_security');
        case 'unsupported':
          return t('auth.error_unsupported');
        case 'secret':
          return t('auth.error_secret');
        case 'rejected':
          return action === 'sign-in' ? t('auth.error_rejected_sign_in') : t('auth.error_rejected_register', { message });
        case 'network':
          return t('auth.error_network');
        default:
          return action === 'sign-in' ? t('auth.error_generic', { message }) : t('auth.error_register_failed', { message });
      }
    },
    [t],
  );
}

/** Default name of a new passkey: "Chrome on macOS" / "macOS 上的 Chrome". */
export function useDefaultPasskeyName() {
  const { t } = useTranslation();
  const { browser, os } = describeDevice(navigator.userAgent);
  if (browser && os) return t('auth.default_name', { browser, os });
  return browser || os || t('auth.default_name_fallback');
}
