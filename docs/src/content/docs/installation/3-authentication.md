---
title: Authentication
description: Protect your Zhang Accounting instance with a login page, a password and passkeys.
---

Without any configuration, everyone who can reach Zhang Accounting can read and change the ledger. Enable at least one sign-in method when the instance is reachable by others, for example when it is deployed on a server.

Two methods are available, and they can be combined:

| `ZHANG_AUTH` | `ZHANG_PASSKEY` | The login page shows |
| --- | --- | --- |
| not set | not set | no login page, the web UI opens directly |
| set | not set | a username and password form |
| not set | set | a **Sign in with passkey** button |
| set | set | both |

Once a method is enabled, opening the web UI shows a login page. After signing in, the browser keeps a session cookie, so you stay signed in until the session expires or you choose **Sign out**.

## Password

The password method uses a username and a password, given in the format `{USERNAME}:{PASSWORD}`, for example `admin:admin888`. The password may contain colons, the first colon separates it from the username.

Enable it with the `--auth` command-line parameter:

```shell
docker run --name zhang kilerd/zhang:latest --auth admin:admin888
```

or with the `ZHANG_AUTH` environment variable:

```shell
docker run --name zhang -e "ZHANG_AUTH=admin:admin888" kilerd/zhang:latest
```

> Note: **Command-line parameters** have priority over environment variables. If both are provided, the command-line parameter is used.

The credentials are entered on the login page, not in a browser popup. Changing `ZHANG_AUTH` signs out every session that was created with the previous credentials.

### Scripts and other HTTP clients

Clients that are not browsers can keep sending the credentials as an HTTP Basic `Authorization` header with every request:

```shell
curl -u admin:admin888 http://localhost:8000/api/info
```

Requests to `/api/*` without a valid session or `Authorization` header are answered with `401` and the JSON body `{"message": "unauthorized"}`.

## Passkeys

Passkeys let you sign in with Face ID, Touch ID, Windows Hello, your phone or a security key instead of a password. Enable them with the `--passkey` command-line parameter or the `ZHANG_PASSKEY` environment variable:

```shell
docker run --name zhang -e "ZHANG_PASSKEY=a-long-random-secret" kilerd/zhang:latest
```

The value of `ZHANG_PASSKEY` is the **registration secret**: whoever knows it can register a passkey without being signed in. Choose a long random value and keep it private. It is not needed to sign in.

### Registering the first passkey

1. Start Zhang Accounting with `ZHANG_PASSKEY` set and open the web UI.
2. As no passkey is registered yet, the login page offers to **Set up a passkey**.
3. Enter the registration secret and, optionally, a name for the passkey.
4. Confirm with your device. You are signed in, and next time you use **Sign in with passkey**.

### Adding more passkeys

Once signed in, open **Settings → Passkeys** and choose **Add passkey** to create a passkey on the current device, no secret needed. To sign in on another device for the first time, use a passkey that syncs to it (for example through iCloud Keychain or Google Password Manager), use your phone through the browser's "use another device" option, or sign in with the password if it is enabled, then add a passkey on that device.

### Removing passkeys

Remove a passkey from **Settings → Passkeys**. The sessions that were signed in with it end immediately. While the password method is disabled, the last passkey cannot be removed, so that you are not locked out.

Removing a passkey from the ledger does not delete it from your device or password manager, delete it there too if you no longer need it.

### Where passkeys are stored

The registered passkeys are stored in `.zhang/passkeys.json` at the root of the ledger, through the configured data source (local files, S3, WebDAV or GitHub), so they survive restarts and redeployments. The file holds public keys only, but keep it with the ledger and include it in your backups. If it is deleted, register a passkey again with the registration secret.

Changing `ZHANG_PASSKEY` does not remove the passkeys that are already registered. If the secret leaked, change it and check the list in **Settings → Passkeys**.

### Domain name and reverse proxies

Browsers only allow passkeys on a domain name served over HTTPS, or on `localhost`. They do not work when the web UI is opened through an IP address such as `http://192.168.1.10:8000`.

A passkey is bound to its **relying party ID**, the domain it was created for. By default, Zhang Accounting uses the host of the request, as seen by the browser: the `X-Forwarded-Host` and `X-Forwarded-Proto` headers of a reverse proxy take precedence over the `Host` header. Most proxies and hosting platforms set them, so a custom domain usually works without any configuration.

When the proxy does not forward them, or to share passkeys between subdomains, set them explicitly:

- `ZHANG_PASSKEY_ORIGIN`: the address the web UI is opened at, for example `https://zhang.example.com`. When it is set, the relying party ID defaults to its host.
- `ZHANG_PASSKEY_RP_ID`: the relying party ID, for example `example.com`. It must be the host of the origin or one of its parent domains.

```shell
docker run --name zhang \
  -e "ZHANG_PASSKEY=a-long-random-secret" \
  -e "ZHANG_PASSKEY_ORIGIN=https://zhang.example.com" \
  kilerd/zhang:latest
```

Passkeys created for one domain cannot be used on another one: after moving the instance to a new domain, sign in with the password or register a passkey again with the registration secret.

## Sessions

Signing in sets a `zhang_session` cookie, valid for 30 days. It is `HttpOnly` and `SameSite=Lax`, and marked `Secure` when the browser reaches the server over HTTPS (as reported by `X-Forwarded-Proto`).

Sessions are signed with a secret key. Set it with the `ZHANG_SESSION_SECRET` environment variable to keep everyone signed in across restarts and redeployments:

```shell
docker run --name zhang \
  -e "ZHANG_AUTH=admin:admin888" \
  -e "ZHANG_SESSION_SECRET=$(openssl rand -hex 32)" \
  kilerd/zhang:latest
```

Without `ZHANG_SESSION_SECRET`, a random key is generated every time the server starts, so a restart signs every browser out. Changing the secret signs every browser out as well.

## Failed sign-in attempts

To slow down password guessing, failed attempts at the password login and at the passkey registration secret are counted. After 5 failed attempts within 15 minutes from the same address, or 50 from all addresses together, further attempts are refused with `429 Too Many Requests` (and a `Retry-After` header) until the 15 minutes have passed, even with the right password. A successful sign-in resets the count of its address. Browsers that are already signed in, and passkey sign-ins, are not affected.

The address is the first one in the `X-Forwarded-For` header set by a reverse proxy, or the address of the connection without one. The counts are kept in memory, so they start over when the server restarts.

## Troubleshooting

- **The web UI opens without a login page**: neither `ZHANG_AUTH` nor `ZHANG_PASSKEY` reached the server. With Docker, check the `-e` options; `ZHANG_AUTH` needs the `{USERNAME}:{PASSWORD}` format.
- **"Incorrect username or password"**: check the value of `ZHANG_AUTH`, the username and the password are both case-sensitive.
- **Signed out after every restart**: set `ZHANG_SESSION_SECRET`.
- **The login page comes back right after signing in**: the browser did not keep the session cookie. Behind a proxy that sets `X-Forwarded-Proto: https`, the cookie is `Secure` and only kept over HTTPS; open the web UI through HTTPS.
- **Passkeys are not offered, or the browser rejects them**: open the web UI through its domain name (or `localhost`) over HTTPS, and when it runs behind a proxy that does not forward the host, set `ZHANG_PASSKEY_ORIGIN`.
- **"The registration secret is incorrect"**: enter the exact value of `ZHANG_PASSKEY` the server was started with.
- **"Too many attempts, try again in N minutes"**: too many failed attempts were made, see [Failed sign-in attempts](#failed-sign-in-attempts). Wait, or restart the server to clear the counts.
