import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loginUrl, returnPath } from './auth-paths.ts';
import {
  assertionToJSON,
  base64UrlToBytes,
  bytesToBase64Url,
  describeDevice,
  isIpAddress,
  registrationToJSON,
  toCreationOptions,
  toRequestOptions,
  withoutNulls,
} from './webauthn.ts';

const bytes = (...values: number[]) => new Uint8Array(values);

describe('base64url', () => {
  it('round-trips every byte value without padding', () => {
    const all = new Uint8Array(256).map((_, i) => i);
    const encoded = bytesToBase64Url(all);
    assert.doesNotMatch(encoded, /[+/=]/);
    assert.deepEqual(base64UrlToBytes(encoded), all);
  });

  it('accepts padded and standard base64 input', () => {
    assert.deepEqual(base64UrlToBytes('-_8='), bytes(0xfb, 0xff));
    assert.deepEqual(base64UrlToBytes('+/8'), bytes(0xfb, 0xff));
    assert.equal(bytesToBase64Url(bytes(0xfb, 0xff)), '-_8');
  });

  it('encodes a view on part of a buffer', () => {
    const view = new Uint8Array([9, 1, 2, 9]).subarray(1, 3);
    assert.equal(bytesToBase64Url(view), bytesToBase64Url(bytes(1, 2)));
  });
});

describe('options', () => {
  it('drops null members recursively', () => {
    assert.deepEqual(withoutNulls({ a: null, b: { c: null, d: 1 }, e: [{ f: null }] }), { b: { d: 1 }, e: [{}] });
  });

  it('decodes the creation challenge, user id and excluded credentials', () => {
    const options = toCreationOptions({
      publicKey: {
        challenge: 'AQID',
        user: { id: 'BAU', name: 'zhang', displayName: 'Zhang' },
        excludeCredentials: [{ type: 'public-key', id: 'Bg' }],
        timeout: 60000,
        extensions: null,
      },
    });
    assert.deepEqual(options.challenge, bytes(1, 2, 3));
    assert.deepEqual(options.user.id, bytes(4, 5));
    assert.equal(options.user.name, 'zhang');
    assert.deepEqual(options.excludeCredentials?.[0].id, bytes(6));
    assert.equal(options.timeout, 60000);
    assert.equal('extensions' in options, false);
  });

  it('decodes the request challenge and allowed credentials, keeping a missing list absent', () => {
    const options = toRequestOptions({ publicKey: { challenge: 'AQID', allowCredentials: [{ type: 'public-key', id: 'Bg' }], rpId: 'localhost' } });
    assert.deepEqual(options.challenge, bytes(1, 2, 3));
    assert.deepEqual(options.allowCredentials?.[0].id, bytes(6));
    assert.equal(options.rpId, 'localhost');
    assert.equal(toRequestOptions({ publicKey: { challenge: 'AQID', allowCredentials: null } }).allowCredentials, undefined);
  });
});

describe('credentials', () => {
  it('serialises an attestation in the webauthn-rs shape', () => {
    const json = registrationToJSON({
      id: 'abc',
      type: 'public-key',
      rawId: bytes(1).buffer,
      response: { attestationObject: bytes(2).buffer, clientDataJSON: bytes(3).buffer },
    });
    assert.deepEqual(json, { id: 'abc', rawId: 'AQ', type: 'public-key', response: { attestationObject: 'Ag', clientDataJSON: 'Aw' } });
  });

  it('serialises an assertion, with a null user handle', () => {
    const credential = {
      id: 'abc',
      type: 'public-key',
      rawId: bytes(1).buffer,
      response: { authenticatorData: bytes(2).buffer, clientDataJSON: bytes(3).buffer, signature: bytes(6).buffer, userHandle: bytes(5).buffer },
    };
    assert.deepEqual(assertionToJSON(credential).response, { authenticatorData: 'Ag', clientDataJSON: 'Aw', signature: 'Bg', userHandle: 'BQ' });
    assert.equal(assertionToJSON({ ...credential, response: { ...credential.response, userHandle: null } }).response.userHandle, null);
  });
});

describe('environment', () => {
  it('recognises IP hosts (not valid passkey RP IDs)', () => {
    assert.equal(isIpAddress('127.0.0.1'), true);
    assert.equal(isIpAddress('[::1]'), true);
    assert.equal(isIpAddress('localhost'), false);
    assert.equal(isIpAddress('zhang.example.com'), false);
  });

  it('names common browsers and systems', () => {
    const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36';
    const iphone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';
    const edge = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.0.0';
    const firefox = 'Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0';
    assert.deepEqual(describeDevice(mac), { browser: 'Chrome', os: 'macOS' });
    assert.deepEqual(describeDevice(iphone), { browser: 'Safari', os: 'iPhone' });
    assert.deepEqual(describeDevice(edge), { browser: 'Edge', os: 'Windows' });
    assert.deepEqual(describeDevice(firefox), { browser: 'Firefox', os: 'Linux' });
  });
});

describe('login return path', () => {
  it('carries the requested page through /login', () => {
    assert.equal(loginUrl({ pathname: '/journals', search: '?page=2', hash: '' }), '/login?next=%2Fjournals%3Fpage%3D2');
    assert.equal(loginUrl({ pathname: '/', search: '', hash: '' }), '/login');
    assert.equal(returnPath('?next=%2Fjournals%3Fpage%3D2'), '/journals?page=2');
  });

  it('refuses other origins and loops', () => {
    for (const next of ['//evil.example', '/\\evil.example', 'https://evil.example', '/login?next=/', '']) {
      assert.equal(returnPath(`?next=${encodeURIComponent(next)}`), '/');
    }
    assert.equal(returnPath(''), '/');
  });
});
