---
title: Plugin
description: "Reference for the plugin directive: its syntax, configuration and the capabilities a plugin can be granted."
sidebar:
  order: 11
---

A `plugin` directive loads a WebAssembly plugin into the ledger. The plugin can transform the ledger while it loads,
report problems in it, or serve pages under `/api/plugins/`. The directive names the module, grants the plugin its
capabilities and passes it its settings.

## Syntax

```text
plugin "<Module>" ["<Argument>" …]
  <key>: "<value>"
  …
```

| Part | Required | Description |
|---|---|---|
| `"<Module>"` | yes | The path of the `.wasm` module, relative to the ledger root. |
| `"<Argument>"` | no | Positional arguments, passed to the plugin as written. |
| `<key>: "<value>"` | no | Metadata lines: the [capabilities](#capabilities) Zhang grants the plugin, and the plugin's own settings. |

A `plugin` directive has no date. It can be written in any file of the ledger.

Plugins are off until the ledger enables them with an option:

```zhang
option "features.plugin" "true"
```

`features.plugins` works too. The value is `true` in any letter case; any other value leaves plugins off. Without the
option, `plugin` directives are ignored and their modules are never read.

## Examples

```zhang
option "features.plugin" "true"

plugin "plugins/fx-rate.wasm" "USD"
  allowed_hosts: "api.frankfurter.dev"
  timeout: "30s"
  base_currency: "USD"

plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

The first plugin may send HTTP requests to `api.frankfurter.dev`, each call into it may run for 30 seconds, and it
receives the setting `base_currency`. The second may read the files under `documents` and the file
`statements/2024.csv`.

## Capabilities

A plugin can do nothing outside its own memory unless its directive grants it. These metadata keys are capabilities:

| Key | Grants | Default |
|---|---|---|
| `allowed_hosts` | HTTP requests to these hosts. Repeat the key for several hosts. | no network access |
| `allowed_paths` | Read-only access to these files and directories of the ledger. Repeat the key for several. | no file access |
| `timeout` | How long one call into the plugin may run. | 60 seconds |
| `seed` | Nothing. Any text, mixed into the seed the plugin derives its random values from. | none |
| `stage` | Where the plugin runs while the ledger loads: `"booked"`, after Zhang has booked the transactions, or `"raw"`, before. | `"booked"` |

**`allowed_hosts`.** Each value is a host name, such as `api.example.com`, matched against the host of each request
the plugin sends. A `*` in it matches any characters, so `*.example.com` grants every sub-domain of `example.com`.
The port and the path are not part of the match.

**`allowed_paths`.** Each value is a file or a directory, relative to the ledger root and written with `/`:

- `"."` grants the whole ledger root. A directory grants everything under it, compared part by part: `documents`
  does not grant `documents-private`.
- Below a granted directory, a hidden name, one starting with `.`, is readable only when a value names it. So `"."`
  does not expose `.git` or `.env`, while `".config"` grants `.config` and everything under it.
- Access is read-only, and nothing outside the ledger root can be read. A file can be at most 16 MiB, and a directory
  listing at most 10,000 entries.
- An empty or absolute value, or one with a `..` part, a backslash or a NUL character, grants nothing and is reported
  as [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta).
- Files are read through the ledger's data source, so this also works for ledgers on S3, WebDAV or GitHub. When a
  plugin reads a file of a local ledger, `zhang serve` reloads the ledger when that file changes.

:::danger[Files and network together]
A plugin granted both `allowed_paths` and `allowed_hosts` can send what it reads off your machine. A plugin already
receives the whole ledger, so grant files and hosts together only to a plugin you would trust with both.
:::

**`timeout`.** Whole seconds (`"90"`), or a whole number with a unit `ms`, `s`, `m` or `h` (`"500ms"`, `"30s"`,
`"2m"`). It must be above zero and at most one day. An invalid value is reported as
[`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta), and the plugin gets the default. If a key is
repeated, its last value counts.

**`stage`.** `"booked"` runs the plugin's processor and mapper after Zhang has booked the transactions: a posting
written without an amount has the amount Zhang inferred, a cost names the per-unit cost and acquisition date of the
lot it matched, and a sale across several lots is one posting per lot, as a Beancount plugin sees them. `"raw"` runs
them before booking, on the transactions as written. Any other value is reported as
[`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta), and the plugin runs `"booked"`. If the key is
repeated, its last value counts. See [the stage order contract](/developers/writing-plugins/#the-stage-order-contract).

**`seed`.** A plugin's seed depends only on its directive: the module as written, the number of `plugin` directives
before it that declare the same module, and the `seed` value. Changing `seed` changes the random values, such as
generated ids, that the plugin derives from it, without moving the directive.

## Behavior

### Settings the plugin receives

The plugin receives a set of string settings:

1. every ledger option, by its key;
2. every metadata key of its directive, with the last value of a repeated key, except `allowed_hosts`. These win over
   an option with the same key;
3. the keys Zhang sets itself: `zhang.abi`, the plugin interface version; `zhang.plugin`, the whole directive as JSON,
   with the positional arguments and every value of every metadata key; `zhang.seed`, the plugin's seed. Keys starting
   with `zhang.` are reserved for Zhang: for these three keys, Zhang's value wins over a metadata key or option of the
   same name.

A plugin can also read settings that change over time from [`custom`](/reference/directives/custom/) directives.

### Loading and order

- Zhang reads the module through the ledger's data source, from the ledger root, every time the ledger loads. When a
  local module changes, `zhang serve` reloads the ledger.
- Plugins run in the order of their `plugin` directives, every time the ledger loads, after Zhang has booked the
  transactions (plugins declared `stage: "raw"` run before that) and before Zhang's own steps: the check of accounts
  that are not open, then [padding](/reference/directives/balance/#padding-with-with-pad), then balance checks. A
  plugin therefore sees the transactions booked, before the padding transactions exist. It never sees a `pad`
  directive: a `balance` a `pad` serves is shown to it as a `balance … with pad` (see
  [the stage order contract](/developers/writing-plugins/#the-stage-order-contract)).
- Declaring the same module twice gives two separate plugins, each with its own settings and seed.
- A module that is missing or cannot be loaded, or a plugin call that fails or runs past its `timeout`, stops the
  ledger from loading. If `zhang serve` is already running, it keeps serving the ledger as it was before the reload.
- The settings page of the web UI lists the loaded plugins. A router plugin serves `/api/plugins/<name>`; see
  [Router plugins](/guides/router-plugins/).

## Errors

| Error | When |
|---|---|
| [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) | A `timeout`, `allowed_paths` or `stage` value is invalid. The error points at the `plugin` directive. |
| [`PluginError`](/reference/error-codes/#pluginerror) | The plugin reports a problem in the ledger. |

## Beancount compatibility

- Beancount's plugins are Python modules, such as `plugin "beancount.plugins.auto_accounts"`. They do not run in Zhang.
  With plugins off, their directives are ignored and the ledger loads. With `features.plugin` on, Zhang looks for a
  module file with that name, and the ledger does not load.
- Beancount does not accept metadata under a `plugin` directive. A ledger that grants capabilities does not pass
  `bean-check`.

## Related

- [Plugins](/guides/plugins/): enabling and trusting plugins.
- [Writing Plugins](/developers/writing-plugins/): building a plugin with the Rust SDK, and the plugin interface.
- [Router plugins](/guides/router-plugins/): plugins that serve pages.
