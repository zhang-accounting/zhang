import { AuthRequestError, finishPasskeyLogin, finishPasskeyRegistration, startPasskeyLogin, startPasskeyRegistration } from '@/api/auth';
import { assertionToJSON, registrationToJSON, toCreationOptions, toRequestOptions } from '@/lib/webauthn';

/**
 * - `dismissed`: the browser prompt was cancelled or timed out (`NotAllowedError` / `AbortError`).
 * - `exists`: this authenticator already holds a passkey for the ledger (`InvalidStateError` on create).
 * - `security`: origin / RP ID mismatch (`SecurityError`), e.g. a domain other than the one the server expects.
 * - `unsupported`: the authenticator cannot satisfy the options (`NotSupportedError`).
 * - `secret`: wrong registration secret (`register/start` 401).
 * - `rejected`: the server did not accept the passkey (`login/finish` / `register/finish` 4xx).
 * - `network`: the server could not be reached. `failed`: anything else (`message` has the detail).
 */
type AttestationCredential = PublicKeyCredential & { response: AuthenticatorAttestationResponse };
type AssertionCredential = PublicKeyCredential & { response: AuthenticatorAssertionResponse };

export type PasskeyErrorKind = 'dismissed' | 'exists' | 'security' | 'unsupported' | 'secret' | 'rejected' | 'network' | 'failed';

export class PasskeyError extends Error {
  readonly kind: PasskeyErrorKind;

  constructor(kind: PasskeyErrorKind, message: string) {
    super(message);
    this.name = 'PasskeyError';
    this.kind = kind;
  }
}

/** `flow`: `register` (with `withSecret` = first passkey from the login page) or `sign-in`. */
function serverError(error: unknown, stage: 'start' | 'finish', flow: 'register' | 'sign-in', withSecret = false): PasskeyError {
  if (error instanceof AuthRequestError) {
    if (error.status === 0) return new PasskeyError('network', error.message);
    if (stage === 'start' && withSecret && error.status === 401) return new PasskeyError('secret', error.message);
    // login/finish: 401 = the assertion did not verify (unknown or removed passkey); other 4xx (expired request) keep the
    // server message. register/finish: any 4xx (attestation refused, already registered) is a rejection with the message.
    const clientError = error.status >= 400 && error.status < 500;
    if (stage === 'finish' && (flow === 'register' ? clientError : error.status === 401)) return new PasskeyError('rejected', error.message);
  }
  return new PasskeyError('failed', error instanceof Error ? error.message : String(error));
}

function browserError(error: unknown): PasskeyError {
  const name = error instanceof Error || error instanceof DOMException ? error.name : '';
  const message = error instanceof Error ? error.message : String(error);
  switch (name) {
    case 'NotAllowedError':
    case 'AbortError':
      return new PasskeyError('dismissed', message);
    case 'InvalidStateError':
      return new PasskeyError('exists', message);
    case 'SecurityError':
      return new PasskeyError('security', message);
    case 'NotSupportedError':
      return new PasskeyError('unsupported', message);
    default:
      return new PasskeyError('failed', message);
  }
}

/**
 * Creates a passkey on this device and registers it; the server signs the browser in. `secret` is the `ZHANG_PASSKEY` value
 * for the first passkey, `null` when already signed in. Throws a `PasskeyError`.
 */
export async function registerPasskey(secret: string | null, name: string | null): Promise<void> {
  let challenge;
  try {
    challenge = await startPasskeyRegistration(secret, name);
  } catch (error) {
    throw serverError(error, 'start', 'register', secret !== null);
  }
  let credential: Credential | null;
  try {
    credential = await navigator.credentials.create({ publicKey: toCreationOptions(challenge.options) });
  } catch (error) {
    throw browserError(error);
  }
  if (!credential) throw new PasskeyError('dismissed', 'no credential');
  try {
    await finishPasskeyRegistration(challenge.state_id, name, registrationToJSON(credential as AttestationCredential));
  } catch (error) {
    throw serverError(error, 'finish', 'register');
  }
}

/** Signs in with a registered passkey (the browser lists the ledger's passkeys). Throws a `PasskeyError`. */
export async function signInWithPasskey(): Promise<void> {
  let challenge;
  try {
    challenge = await startPasskeyLogin();
  } catch (error) {
    throw serverError(error, 'start', 'sign-in');
  }
  let credential: Credential | null;
  try {
    credential = await navigator.credentials.get({ publicKey: toRequestOptions(challenge.options) });
  } catch (error) {
    throw browserError(error);
  }
  if (!credential) throw new PasskeyError('dismissed', 'no credential');
  try {
    await finishPasskeyLogin(challenge.state_id, assertionToJSON(credential as AssertionCredential));
  } catch (error) {
    throw serverError(error, 'finish', 'sign-in');
  }
}
