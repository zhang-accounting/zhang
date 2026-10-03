/**
 * WebAuthn JSON <-> browser conversion for the zhang-server passkey endpoints (webauthn-rs). The server sends
 * `CreationChallengeResponse` / `RequestChallengeResponse` JSON (`{ publicKey }`, binary fields base64url) and reads the
 * credential back as `{ id, rawId, type, response: { ... } }` with base64url fields, the shape of the webauthn-rs tutorial.
 * Dependency-free (no `@/` imports) so `node --test` can load it.
 */

interface CredentialDescriptorJSON {
  type: string;
  id: string;
  transports?: string[] | null;
}

export interface CreationOptionsJSON {
  publicKey: {
    challenge: string;
    user: { id: string; name: string; displayName: string };
    excludeCredentials?: CredentialDescriptorJSON[] | null;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface RequestOptionsJSON {
  publicKey: {
    challenge: string;
    allowCredentials?: CredentialDescriptorJSON[] | null;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

/** base64url (RFC 4648 §5) to bytes; padding and the standard `+` / `/` alphabet are accepted too. */
export function base64UrlToBytes(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.replace(/-/g, '+').replace(/_/g, '/').replace(/=+$/, '');
  const binary = atob(base64 + '='.repeat((4 - (base64.length % 4)) % 4));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Bytes to unpadded base64url. */
export function bytesToBase64Url(data: ArrayBuffer | ArrayBufferView): string {
  const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Drops `null` members (serde `Option::None`): WebIDL rejects `null` for sequences such as `excludeCredentials`. */
export function withoutNulls<T>(value: T): T {
  if (Array.isArray(value)) return value.map((item) => withoutNulls(item)) as T;
  if (value === null || typeof value !== 'object') return value;
  const result: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    if (item !== null && item !== undefined) result[key] = withoutNulls(item);
  }
  return result as T;
}

const toDescriptor = (descriptor: CredentialDescriptorJSON) =>
  ({ ...descriptor, id: base64UrlToBytes(descriptor.id) }) as unknown as PublicKeyCredentialDescriptor;

/** `options.publicKey` of `register/start`, ready for `navigator.credentials.create({ publicKey })`. */
export function toCreationOptions(options: CreationOptionsJSON): PublicKeyCredentialCreationOptions {
  const publicKey = withoutNulls(options.publicKey);
  return {
    ...publicKey,
    challenge: base64UrlToBytes(publicKey.challenge),
    user: { ...publicKey.user, id: base64UrlToBytes(publicKey.user.id) },
    excludeCredentials: publicKey.excludeCredentials?.map(toDescriptor),
  } as unknown as PublicKeyCredentialCreationOptions;
}

/** `options.publicKey` of `login/start`, ready for `navigator.credentials.get({ publicKey })`. */
export function toRequestOptions(options: RequestOptionsJSON): PublicKeyCredentialRequestOptions {
  const publicKey = withoutNulls(options.publicKey);
  return {
    ...publicKey,
    challenge: base64UrlToBytes(publicKey.challenge),
    allowCredentials: publicKey.allowCredentials?.map(toDescriptor),
  } as unknown as PublicKeyCredentialRequestOptions;
}

interface AttestationCredential {
  id: string;
  type: string;
  rawId: ArrayBuffer;
  response: { attestationObject: ArrayBuffer; clientDataJSON: ArrayBuffer };
}

interface AssertionCredential {
  id: string;
  type: string;
  rawId: ArrayBuffer;
  response: { authenticatorData: ArrayBuffer; clientDataJSON: ArrayBuffer; signature: ArrayBuffer; userHandle: ArrayBuffer | null };
}

/** `RegisterPublicKeyCredential` JSON for `register/finish`. */
export function registrationToJSON(credential: AttestationCredential) {
  return {
    id: credential.id,
    rawId: bytesToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      attestationObject: bytesToBase64Url(credential.response.attestationObject),
      clientDataJSON: bytesToBase64Url(credential.response.clientDataJSON),
    },
  };
}

/** `PublicKeyCredential` JSON for `login/finish`. */
export function assertionToJSON(credential: AssertionCredential) {
  const { response } = credential;
  return {
    id: credential.id,
    rawId: bytesToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      authenticatorData: bytesToBase64Url(response.authenticatorData),
      clientDataJSON: bytesToBase64Url(response.clientDataJSON),
      signature: bytesToBase64Url(response.signature),
      userHandle: response.userHandle ? bytesToBase64Url(response.userHandle) : null,
    },
  };
}

/** Why passkeys cannot be used on this page, or `null` when they can. */
export type PasskeyUnavailableReason = 'insecure' | 'unsupported' | 'ip-address';

/** IPv4 / bracketed IPv6 host: not a valid WebAuthn RP ID, browsers throw `SecurityError`. */
export function isIpAddress(hostname: string) {
  return /^\d{1,3}(\.\d{1,3}){3}$/.test(hostname) || hostname.startsWith('[') || hostname.includes(':');
}

export function passkeyUnavailableReason(): PasskeyUnavailableReason | null {
  if (typeof window === 'undefined') return 'unsupported';
  if (!window.isSecureContext) return 'insecure';
  if (typeof window.PublicKeyCredential === 'undefined' || !navigator.credentials?.create) return 'unsupported';
  if (isIpAddress(window.location.hostname)) return 'ip-address';
  return null;
}

/** Browser + OS of a user agent, for the default passkey name ("Chrome on macOS"). */
export function describeDevice(userAgent: string): { browser: string; os: string } {
  const browser = /Edg(e|A|iOS)?\//.test(userAgent)
    ? 'Edge'
    : /OPR\/|Opera/.test(userAgent)
      ? 'Opera'
      : /Firefox\/|FxiOS\//.test(userAgent)
        ? 'Firefox'
        : /Chrome\/|CriOS\/|Chromium\//.test(userAgent)
          ? 'Chrome'
          : /Safari\//.test(userAgent)
            ? 'Safari'
            : '';
  const os = /iPhone/.test(userAgent)
    ? 'iPhone'
    : /iPad/.test(userAgent)
      ? 'iPad'
      : /Android/.test(userAgent)
        ? 'Android'
        : /CrOS/.test(userAgent)
          ? 'ChromeOS'
          : /Mac OS X|Macintosh/.test(userAgent)
            ? 'macOS'
            : /Windows/.test(userAgent)
              ? 'Windows'
              : /Linux/.test(userAgent)
                ? 'Linux'
                : '';
  return { browser, os };
}
