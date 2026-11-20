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

Copy the plugin's file, a `.wasm` module or a `.lua` script, into your ledger directory, for example into `plugins/`, and declare it:

```zhang
option "features.plugin" "true"

plugin "plugins/large_expense.wasm"
  threshold: "500 CNY"
```

- The first value is the module. Its path is relative to the ledger's root directory, whichever file the directive is in. For a ledger stored on [S3, WebDAV or GitHub](/deployment/data-sources/s3/), Zhang reads the module from that storage too, so keep it next to your ledger files.
- Values after the module are arguments for the plugin, and the metadata lines hold its settings (here `threshold`). Which ones a plugin understands is up to its author. The plugin also receives the ledger's options.
- Some metadata keys are not settings but **capabilities** you grant the plugin, described [below](#grant-capabilities).
- Plugins run in the order they are declared. A module declared twice runs twice.
- Zhang reads each module from the ledger's storage every time the ledger loads, and keeps no copy of it. When a module on the local disk changes, `zhang serve` reloads the ledger.

## What plugins do

A plugin declares what it is, and can be several of these at once:

- A **processor** receives every entry of the ledger and returns the entries Zhang goes on with. It can add, change or remove entries, or only check them and report problems.
- A **mapper** does the same one entry at a time.
- A **router** serves pages and data under `/api/plugins/{name}`, such as a custom report. See [Router Plugins](/guides/router-plugins/). A router can be written as a Lua script, with nothing to compile: see [Lua Plugins](/guides/lua-plugins/).

What a processor or mapper changes exists only while the ledger is loaded. Your files stay as they are.

## Grant capabilities

Outside its own memory, a plugin can do nothing that its directive does not grant, and never more than the server allows (see [below](#what-the-server-allows)):

| Metadata | Lets the plugin | Without it |
|---|---|---|
| `allowed_hosts` | make HTTP requests to these hosts, among those the server allows. Repeat the key for several hosts. | no network access |
| `allowed_paths` | read these files and directories of the ledger. Repeat the key for several. | no file access |
| `allowed_writes` | add entries of these kinds to your ledger files, `"transaction"` or `"open"`, when it serves a request (a recording assistant, say), and only when `zhang serve` runs with `--plugins write` (`ZHANG_PLUGINS=write`). Repeat the key for both. | no writes |
| `timeout` | run each call for up to this long, at most what the server allows: whole seconds (`"90"`) or a number with a unit `ms`, `s`, `m` or `h` (`"500ms"`, `"2m"`) | 60 seconds, at most the server's limit (30 seconds unless set) |
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

About `allowed_writes`:

- It only works for a router plugin that declares it writes (the plugin's author decides), and only for the kinds you grant: a plugin granted `"transaction"` cannot open an account.
- An entry a plugin adds goes through the same checks and into the same file as one you record in the web UI, and is appended as text, exactly as the plugin previewed it: nothing is edited or deleted, and Zhang adds nothing of its own. The ledger reloads after each write, as after one of yours, and the server logs one line per write.
- A plugin written for people shows you each entry before adding it. Grant writes only to a plugin you trust.

A value Zhang cannot use, such as `timeout: "soon"`, is reported as a [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) error on the `plugin` directive, and the plugin gets the default.

About `allowed_hosts`: a request only goes to a public address. A host that resolves to the machine itself, the private network or a cloud metadata service (`169.254.169.254`, Azure's `168.63.129.16`) is refused, and redirects are not followed. Any port of an allowed host can be reached, and the server's own public address counts as public, so grant only the hosts a plugin needs. At most 8 name lookups of plugins run at once, and at most 2 of any one plugin: when the name server of a plugin's host does not answer, that plugin's further requests fail rather than pile up, and the other plugins keep resolving names.

## What the server allows

Whoever can change your ledger files, through the web UI's file editor, a Git repository or a storage bucket, can write `plugin` directives. So the server, not the ledger, has the last word, with options of `zhang serve`:

| Setting | Default | What it does |
|---|---|---|
| `--plugins` / `ZHANG_PLUGINS` | `read` | `off` loads no plugin; `read` runs plugins, which change nothing; `write` also lets a plugin write to the ledger where its directive grants it. |
| `ZHANG_PLUGIN_ALLOWED_HOSTS` | none | The hosts plugins may reach, separated by commas, such as `api.frankfurter.dev,*.example.com`; `*` allows any host. A plugin reaches a host only if its `allowed_hosts` and this setting both allow it. |
| `ZHANG_PLUGIN_MAX_TIMEOUT` | `30s` | The longest one call into a plugin may run. |
| `ZHANG_PLUGIN_MAX_MEMORY` | `256MiB` | The most memory one plugin instance may use. |

So by default plugins run read-only, without any network, for at most 30 seconds a call. When a directive grants more, the plugin still loads with what the server allows, and the error list shows a [`PluginCapabilityReduced`](/reference/error-codes/#plugincapabilityreduced) error on the directive that names the setting to change. With `--plugins off`, each `plugin` directive shows a [`PluginsDisabled`](/reference/error-codes/#pluginsdisabled) error instead.

For example, to let the exchange-rate plugin above fetch its rates, start the server with:

```shell
ZHANG_PLUGIN_ALLOWED_HOSTS=api.frankfurter.dev zhang serve /path/to/ledger
```

:::danger[Files and network together]
A plugin with both `allowed_paths` and `allowed_hosts` can send what it reads off your machine. A plugin already receives your whole ledger, so grant both only to a plugin you would trust with both.
:::

## Decide which plugins to trust

Every plugin receives your **whole ledger**: every entry and every option. It can change what Zhang shows you, by changing entries before Zhang checks and displays them.

What the sandbox prevents:

- network access, except to the public addresses of the hosts in `allowed_hosts` that the server allows;
- reading files, except those `allowed_paths` grants inside the ledger root;
- writing files, including your ledger files, except appending the kinds of entries `allowed_writes` grants, when the server allows writes;
- running longer than its `timeout`;
- using more memory than the server allows, 256 MiB per plugin instance unless the `ZHANG_PLUGIN_MAX_MEMORY` environment variable of `zhang serve` sets another size (such as `512MiB`).

A router plugin's pages are served from Zhang's own address, but [sandboxed](/guides/router-plugins/#pages-are-sandboxed): a script on such a page cannot use your browser's session to call Zhang's API.

Only declare plugins you trust, ideally ones whose source you can read, and grant each one only what it needs.

## See which plugins are loaded

- The **Settings** page lists the loaded plugins, in the order they run, with their name, version and types. A router plugin has a link to its pages.
- `GET /api/plugins` returns the same list as JSON, with the hosts each plugin may reach (its `allowed_hosts` within what the server allows), the route of each router plugin, and the writes each plugin asked for (`writes_requested`) and was granted within what the server allows (`writes_granted`). To review `allowed_paths` and `timeout`, read the `plugin` directives of your ledger.

## When a plugin goes wrong

- A problem a plugin reports appears in the ledger's error list as a [`PluginError`](/reference/error-codes/#pluginerror), with the plugin's name and message, on the entry the plugin points at or on its `plugin` directive. The ledger still loads.
- A plugin that fails, by returning an error, crashing, running past its timeout or out of memory, stops the whole ledger from loading, and so does a module that cannot be loaded, such as a path where no module is. When Zhang starts, it exits with the error. While it is running, it keeps serving the last version that loaded and only logs the error: the web UI shows no error. A timeout message says which call ran too long; raise the plugin's `timeout` if it needs more time. An out-of-memory message names the limit; raise `ZHANG_PLUGIN_MAX_MEMORY` if the plugin needs more, for example to process a very large ledger.

## Plugins, pads and balance assertions

Zhang runs your plugins first, then checks that every account is open, then fills the pads, then checks the balance assertions. So:

- Transactions a plugin adds count in the amounts pads fill up to and in the balances assertions check. A posting a plugin adds to an account that is not open is reported like one you wrote.
- A plugin sees your `balance` and `balance … with pad` directives, but not the padding transactions, which are added after it runs. It does not see `pad` directives: a `balance` a `pad` serves is shown to it as a `balance … with pad`. See [Writing plugins](/developers/writing-plugins/#the-stage-order-contract).
- A plugin sees transactions as you wrote them: a missing amount is not filled in yet, and sales are not matched to lots.
- An `option` or `plugin` directive a plugin adds has no effect.

See [Balances and Padding](/guides/balances/) for how pads and assertions work.

## Beancount plugins

Beancount's plugins are Python code, which Zhang does not run. In a beancount ledger without `features.plugin`, Zhang ignores lines such as `plugin "beancount.plugins.auto_accounts"`. With `features.plugin` on, it tries to load them as WASM modules, and the ledger fails to load. See [Coming from Beancount](/getting-started/from-beancount/#compatibility).
