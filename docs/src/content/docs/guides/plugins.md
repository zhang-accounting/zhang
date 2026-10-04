---
title: Using Plugins
description: Enable WASM plugins in a ledger with the plugin directive, grant them capabilities and decide which plugins to trust.
sidebar:
  order: 7
---

Plugins add behavior to Zhang without changing Zhang itself. A plugin can generate entries (recurring transactions, split expenses), check your ledger against your own rules, or serve a report page of its own. Plugins are WebAssembly (WASM) modules that Zhang runs in a sandbox while it loads your ledger.

This guide is for using plugins. To write one, see [Writing Plugins](/developers/writing-plugins/). The exact syntax of the directive is in [Plugin](/reference/directives/plugin/).

## Enable plugins

Plugins are off by default. Turn them on in your ledger:

```zhang
option "features.plugin" "true"
```

`features.plugins` works too. Without the option, Zhang ignores every `plugin` directive and never reads the modules.

## Add a plugin

Copy the plugin's `.wasm` file into your ledger directory, for example into `plugins/`, and declare it:

```zhang
option "features.plugin" "true"

plugin "plugins/large_expense.wasm"
  threshold: "500 CNY"
```

- The first value is the module. Its path is relative to the ledger's root directory, whichever file the directive is in. For a ledger stored on [S3, WebDAV or GitHub](/deployment/data-sources/s3/), Zhang reads the module from that storage too, so keep it next to your ledger files.
- Values after the module are arguments for the plugin, and the metadata lines hold its settings (here `threshold`). Which ones a plugin understands is up to its author. The plugin also receives the ledger's options.
- Some metadata keys are not settings but **capabilities** you grant the plugin, described [below](#grant-capabilities).
- Plugins run in the order they are declared. A module declared twice runs twice.
- Zhang copies each module to `.cache/plugins/` in the directory it runs in, and loads it from there. When a module on the local disk changes, `zhang serve` reloads the ledger.

## What plugins do

A plugin declares what it is, and can be several of these at once:

- A **processor** receives every entry of the ledger and returns the entries Zhang goes on with. It can add, change or remove entries, or only check them and report problems.
- A **mapper** does the same one entry at a time.
- A **router** serves pages and data under `/api/plugins/{name}`, such as a custom report. See [Router Plugins](/guides/router-plugins/).

What a processor or mapper changes exists only while the ledger is loaded. Your files stay as they are.

## Grant capabilities

Outside its own memory, a plugin can do nothing that its directive does not grant:

| Metadata | Lets the plugin | Without it |
|---|---|---|
| `allowed_hosts` | make HTTP requests to these hosts. Repeat the key for several hosts. | no network access |
| `allowed_paths` | read these files and directories of the ledger. Repeat the key for several. | no file access |
| `timeout` | run each call for up to this long: whole seconds (`"90"`) or a number with a unit `ms`, `s`, `m` or `h` (`"500ms"`, `"2m"`), at most a day | 60 seconds |
| `seed` | nothing more. Any text: it changes the seed the plugin derives random values from. | the default seed |

```zhang
plugin "plugins/fx-rates.wasm"
  allowed_hosts: "api.frankfurter.dev"
  timeout: "30s"

plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

About `allowed_paths`:

- Each value is a file or a directory, relative to the ledger root and written with `/`. `"."` is the whole root.
- Access is read-only, and limited to the ledger root, on the local disk and on remote storage alike.
- Names starting with a dot are hidden: below a granted directory, a plugin can read `.git` or `.env` only if a value names them.
- A value that is absolute or contains `..` grants nothing.
- Only processors and mappers can read files. On a local ledger, `zhang serve` reloads when a file a plugin read changes.

A value Zhang cannot use, such as `timeout: "soon"`, is reported as a [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) error on the `plugin` directive, and the plugin gets the default.

:::danger[Files and network together]
A plugin with both `allowed_paths` and `allowed_hosts` can send what it reads off your machine. A plugin already receives your whole ledger, so grant both only to a plugin you would trust with both.
:::

## Decide which plugins to trust

Every plugin receives your **whole ledger**: every entry and every option. It can change what Zhang shows you, by changing entries before Zhang checks and displays them.

What the sandbox prevents:

- network access, except to the hosts in `allowed_hosts`;
- reading files, except those `allowed_paths` grants inside the ledger root;
- writing files, including your ledger files;
- running longer than its `timeout`.

A router plugin's pages are served from Zhang's own address. A script on such a page can call Zhang's API, including the endpoints that change your ledger files, with your browser's session.

Only declare plugins you trust, ideally ones whose source you can read, and grant each one only what it needs.

## See which plugins are loaded

- The **Settings** page lists the loaded plugins, in the order they run, with their name, version and types. A router plugin has a link to its pages.
- `GET /api/plugins` returns the same list as JSON, with the `allowed_hosts` each plugin was granted and the route of each router plugin. To review `allowed_paths` and `timeout`, read the `plugin` directives of your ledger.

## When a plugin goes wrong

- A problem a plugin reports appears in the ledger's error list as a [`PluginError`](/reference/error-codes/#pluginerror), with the plugin's name and message, on the entry the plugin points at or on its `plugin` directive. The ledger still loads.
- A plugin that fails, by returning an error, crashing or running past its timeout, stops the whole ledger from loading, and so does a module that cannot be loaded, such as a path where no module is. When Zhang starts, it exits with the error. While it is running, it keeps serving the last version that loaded and only logs the error: the web UI shows no error. A timeout message says which call ran too long; raise the plugin's `timeout` if it needs more time.

## Plugins, pads and balance assertions

Zhang runs your plugins first, then checks that every account is open, then fills the pads, then checks the balance assertions. So:

- Transactions a plugin adds count in the amounts pads fill up to and in the balances assertions check. A posting a plugin adds to an account that is not open is reported like one you wrote.
- A plugin sees your `balance` and `balance … with pad` directives, but not the padding transactions, which are added after it runs. It does not see `pad` directives: a `balance` a `pad` serves is shown to it as a `balance … with pad`. See [Writing plugins](/developers/writing-plugins/#the-stage-order-contract).
- A plugin sees transactions as you wrote them: a missing amount is not filled in yet, and sales are not matched to lots.
- An `option` or `plugin` directive a plugin adds has no effect.

See [Balances and Padding](/guides/balances/) for how pads and assertions work.

## Beancount plugins

Beancount's plugins are Python code, which Zhang does not run. In a beancount ledger without `features.plugin`, Zhang ignores lines such as `plugin "beancount.plugins.auto_accounts"`. With `features.plugin` on, it tries to load them as WASM modules, and the ledger fails to load. See [Coming from Beancount](/getting-started/from-beancount/#compatibility).
