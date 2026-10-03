---
title: Writing Plugins
description: How to write WASM plugins for Zhang with the Rust SDK, and the plugin ABI v1 reference — the plugin directive and its capabilities, the stage order, config keys, host functions, determinism, custom-directive config and router plugins.
---

A plugin is a WebAssembly module that Zhang runs while it loads a ledger, or when an HTTP request reaches it. With a plugin you can:

- **transform** the ledger: add, change or remove directives, e.g. generate recurring transactions, split expenses or tag entries;
- **validate** it: report problems in the ledger's error list without changing anything;
- **serve** pages and data: answer HTTP requests under `/api/plugins/{name}`, e.g. a custom report.

Plugins are built with [Extism](https://extism.org/), so any language with an Extism plug-in kit works. This guide uses the Rust SDK, `zhang-plugin-sdk`, and documents the ABI underneath it for everyone else.

:::caution[Trust]
A plugin sees your whole ledger. Only declare plugins you trust, and grant each one only the capabilities it needs.
:::

## Enabling plugins

Plugins are off by default. Turn them on in the ledger:

```zhang
option "features.plugin" "true"
```

`features.plugins` works too. Without the option, `plugin` directives are ignored and their modules are never read.

## The `plugin` directive

```zhang
plugin "plugins/guard.wasm" "strict"
  threshold: "500 CNY"
  allowed_paths: "guard"
  timeout: "10s"
```

- The first value is the module. Zhang reads it through the ledger's data source, so the path is relative to the ledger root, and works the same for ledgers on S3, WebDAV or GitHub.
- The following values are **positional arguments** (here `"strict"`). Beancount's `plugin "module" "config"` passes its config string this way.
- The meta lines hold the **capabilities** Zhang grants the plugin (below) and the plugin's own **config** (here `threshold`).

Meta keys starting with `zhang.` are reserved for values Zhang sets.

### Capabilities

A plugin can do nothing outside its own memory unless its directive grants it.

| Meta | Grants | Default |
|---|---|---|
| `allowed_hosts` | HTTP requests to these hosts. Repeat the key for several hosts. | no network at all |
| `allowed_paths` | read-only access to these files and directories of the ledger. Repeat the key for several. | no file access |
| `timeout` | how long one call into the plugin may run: whole seconds (`"90"`) or a number with a unit `ms`, `s`, `m` or `h` (`"500ms"`, `"2m"`), at most a day | 60 seconds |
| `seed` | nothing; any text, mixed into the plugin's [seed](#determinism) | none |

An invalid `timeout` or `allowed_paths` value is reported as a [`ParseInvalidMeta`](/user-guide/error-code/#parseinvalidmeta) error on the directive, and the plugin gets the default.

#### `allowed_paths`

```zhang
plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

- Each value is a file or a directory, relative to the ledger root and written with `/`. `"."` grants the whole root.
- A directory grants everything under it, compared component by component: `documents` does not grant `documents-private/`.
- **The dot rule:** below a grant, a hidden name (one starting with `.`) is readable only when a value names it. So `"."` does not expose `.git/config`, `.env` or `.cache/`, while `".config"` grants `.config/…`, and `"documents/.receipts"` grants exactly that directory. Listings leave hidden entries out.
- A value that is empty or absolute, or that holds a `..` component, a NUL or a backslash, grants nothing.
- Access is **read-only**, and nothing outside the ledger root can be reached: on a local disk, a symlink is followed only while it stays inside the grant. A file can be at most 16 MiB, and a listing at most 10 000 entries.
- Only a processor or mapper reads files. Every file or directory a plugin reads is recorded, and `zhang serve` reloads a local ledger when one of them changes, appears or disappears.

:::danger[Files and network together]
`allowed_paths` together with `allowed_hosts` lets a plugin send what it reads off your machine. A plugin already receives the whole ledger, so grant files and hosts together only to a plugin you would trust with both.
:::

## Plugin types

A plugin declares what it is in its `supported_type` export, and can be several things at once.

| Type | Export | Runs | Input → output |
|---|---|---|---|
| `Processor` | `processor` | once per load | the whole directive stream → the new stream |
| `Mapper` | `mapper` | once per directive, in one plugin instance for the whole stream | one directive → the directives replacing it (none, itself, or several) |
| `Router` | `router` | once per HTTP request, each in a fresh instance | the request → the response |

A plugin that is both a processor and a mapper runs its processor first. A type Zhang does not know is ignored with a warning, so a plugin written for a newer Zhang still loads.

## The stage order contract

While Zhang loads a ledger, the directive stream runs through these stages, in this order:

1. **your plugins**, in the order their `plugin` directives are declared;
2. **active accounts**: postings to accounts that are not open are reported;
3. **pad**: each `pad` and `balance … with pad` adds the padding transaction (flag `P`) its assertion needs;
4. **balance check**: each balance assertion is checked. It books nothing: a failing one is an error.

Then the transactions are booked and the ledger is built.

What a plugin sees:

- **The full stream**, sorted by date: undated directives (`option`, `plugin`, `include`, comments) first; within one date, `open` and `commodity`, then balance directives, then everything else. Zhang re-sorts the stream after every stage, so a plugin may return directives in any order of dates; directives of the same date and kind keep the order the plugin returns them in, which is the order of their day.
- Every directive kind, including `custom`, `option` and `plugin` directives. Options and plugins are applied before the stages run, so an `option` or `plugin` directive a plugin adds has no effect.
- **Transactions as written, before booking.** A posting written without an amount has no amount yet, and costs are not matched to lots. A later version of Zhang will offer plugins a booked view as well; this guide will say so when it lands.
- The `balance` directives themselves, but not the padding transactions (flag `P`) the pad stage creates: it runs after the plugins.
- **No `pad` directives.** ABI v1 predates the [`pad` directive](/directives/2-account/#pads), and a plugin built against an older `zhang-ast` cannot read it. So Zhang sets every `pad` aside before it calls a plugin. A `balance` that a `pad` serves is shown to the plugin as the `balance … with pad` it was before Zhang had `pad`, with the pad's account: a plugin sees the stream a Beancount ledger gave it before. What the plugin returns is its word, and Zhang puts each `pad` back only where it pads what the plugin returned:
  - a `pad` whose `balance … with pad`s all come back as `balance … with pad`, of one account from one pad account, is put back with that account and pad account, so a plugin may rename either or change the pad account. Its balances turn back into `balance`s, with their tolerance, and keep everything else the plugin changed in them. A plugin that changes nothing gets exactly the stream it was given;
  - a `pad` one of whose `balance … with pad`s the plugin dropped, turned into a plain `balance`, or gave another account or pad account than the others, is left out, and so is a `pad` that, put back, would serve other balances than the ones it stood for (when a plugin moves one to another date, or adds a balance of the account before one). Every `balance … with pad` the plugin returned then pads its own assertion, as a `balance … with pad` does;
  - a `pad` that serves no balance is invisible to a plugin, which cannot change or drop it. It is put back as it is, and must still serve none: put back where it would serve one, it is left out.

  A `pad` put back serves only the balances it stood for. Any other `balance` the plugin returns, such as one it adds or one it turned into a plain `balance`, is not padded by a `pad` it could not see, and is checked as it is.

  Pads are not visible to plugins yet; exposing them is future ABI work.

**Why plugins run before pad and balance check.** Running plugins first means a pad is sized after every transaction a plugin adds, so the account always ends at the amount you wrote. Beancount runs `pad` before plugins and re-checks `balance` after them; Zhang has no second check, so if pad ran first, a transaction a plugin adds to a padded account would silently move the balance. For a working beancount ledger the padded amount is the same either way. Only a plugin that inspects the padding transactions themselves notices the difference, and it still sees the `balance` directive.

## Quickstart with the Rust SDK

`zhang-plugin-sdk` lives in the Zhang repository and is versioned with it; it is not on crates.io yet. Pin it to the Zhang release you run: the directives cross the boundary in `zhang-ast`'s JSON shape, and a plugin built against an older `zhang-ast` cannot read a directive kind a newer Zhang added. Zhang keeps the kinds added since ABI v1 (the `pad` directive) away from v1 plugins.

1. Create a library crate and make it a `cdylib`:

   ```toml
   # Cargo.toml
   [package]
   name = "large-expense"
   version = "0.1.0"
   edition = "2021"

   [lib]
   crate-type = ["cdylib"]

   [dependencies]
   zhang-plugin-sdk = { git = "https://github.com/zhang-accounting/zhang", tag = "vX.Y.Z" }
   ```

2. Write the plugin. `plugin!` exports `name`, `version`, `supported_type` (derived from the handlers you give) and the handlers, and does the JSON for you:

   ```rust
   // src/lib.rs
   use zhang_plugin_sdk::config::Config;
   use zhang_plugin_sdk::{custom, errors, plugin, Directive, Error, Stream};

   const NAME: &str = "large-expense";

   plugin! {
       name: NAME,
       version: env!("CARGO_PKG_VERSION"),
       processor: process,
   }

   fn process(stream: Stream) -> Result<Stream, Error> {
       // flat config, the directive's meta, and `custom "large-expense" …` directives
       let config = Config::load().with_custom(custom::entries(NAME, &stream));
       for directive in &stream {
           let Directive::Transaction(txn) = &directive.data else { continue };
           let date = txn.date.naive_date();
           let Some(threshold) = config.resolve("threshold", date, Some(&txn.meta)) else { continue };
           let threshold = threshold.amount(0)?;
           for posting in &txn.postings {
               if let Some(units) = &posting.units {
                   if units.commodity == threshold.commodity && units.number > threshold.number {
                       errors::emit_error_at(&directive.span, format!("{} is a large expense", posting.account.name()), [("rule", "threshold")]);
                   }
               }
           }
       }
       Ok(stream)
   }
   ```

3. Build it for `wasm32-unknown-unknown`:

   ```sh
   rustup target add wasm32-unknown-unknown
   cargo build --release --target wasm32-unknown-unknown
   cp target/wasm32-unknown-unknown/release/large_expense.wasm ~/ledger/plugins/
   ```

4. Declare it in the ledger:

   ```zhang
   option "features.plugin" "true"
   plugin "plugins/large_expense.wasm"
     threshold: "500 CNY"

   2024-07-01 custom "large-expense" "threshold" 300 CNY
   ```

What the SDK offers:

| Module | For |
|---|---|
| `plugin!` | the exports, with typed handlers: `fn(Stream) -> Result<Stream, Error>`, `fn(Spanned<Directive>) -> Result<Stream, Error>`, `fn(Request) -> Result<Response, Error>` |
| `config` | `Config::load()`, `get` (flat keys), `abi`, `plugin` (arguments and multi-valued meta), `meta`, `option`, `seed`, `resolve`, and `Values` to parse numbers, amounts, dates, booleans and accounts |
| `custom` | `entries(name, &stream)` and `latest(…)` for [`custom` config](#config-in-custom-directives) |
| `clock` | `now()`, `today()`, `rng()`, `rng_for(&directive)` |
| `fs` | `read_file`, `read_to_string`, `list_dir` |
| `errors` | `emit_error(message)`, `emit_error_at(span, message, metas)` |
| `prices` | `PriceMap::from_stream(&stream)`, `rate(base, quote, date)`, `convert(amount, target, date)` for [exchange rates](#exchange-rates) |
| `router` | `Request`, `Response`, `query(bql)`, `ledger_info()` |

On a native target the SDK still compiles: `plugin!` exports nothing and host functions answer `unavailable`, so `cargo test` runs your plugin logic without Zhang, with a `Config::from_map(...)` standing in for the host's config. Two complete plugins, a processor and a router, live in [`zhang-plugin-sdk/examples`](https://github.com/zhang-accounting/zhang/tree/main/zhang-plugin-sdk/examples); Zhang's own tests build and run them.

## Determinism

A ledger should load the same way every time. Zhang cannot enforce it, so it is a contract:

- **Read the time from Zhang**, with the `zhang_now` host function (`clock::now()` / `clock::today()` in the SDK). Zhang reads its clock once per load, so every plugin sees the same instant, in the ledger's timezone. Reading it makes the ledger depend on the date, and `zhang serve` reloads such a ledger at midnight.
- **Derive randomness from the seed**, the `zhang.seed` config (`clock::rng()` and `clock::rng_for(&directive)` in the SDK). The seed depends only on the plugin's directive — its module path, its position among directives of the same module, and its `seed` meta — so generated ids and links stay the same on every reload. `rng_for` also mixes in the directive's text, so an id does not move when other parts of the file change.
- **Build for `wasm32-unknown-unknown`.** A plugin built for WASI can read the host's real clock and entropy, which Zhang cannot intercept; such a plugin is not reproducible.

## Config in `custom` directives

Config that changes over time belongs in the ledger, as dated `custom` directives whose first value is the plugin's name:

```zhang
2024-01-01 custom "large-expense" "threshold" 100 CNY
2024-07-01 custom "large-expense" "threshold" "150 CNY"
```

An entry dated 2024-03-05 sees the threshold of 2024-01-01; one dated 2024-08-01 the one of 2024-07-01. Values stay strings: `100 CNY` arrives as the two values `"100"` and `"CNY"`, and the SDK's `Values::amount` reads both shapes.

`Config::resolve(key, date, entry_meta)` looks a setting up in this order, the first one holding the key winning:

1. the metadata of the entry being processed;
2. the latest `custom "<plugin>" "<key>" …` dated on or before the entry (of several on one day, the last);
3. the meta of the plugin's `plugin` directive;
4. a ledger option of that name.

Only a processor sees the whole stream, so only a processor can read `custom` config; a mapper sees one directive at a time.

## Exchange rates

Zhang never precomputes prices for plugins. A processor that needs exchange rates builds them from the stream it receives, with the SDK's `PriceMap`:

```rust
use zhang_plugin_sdk::prices::PriceMap;

let prices = PriceMap::from_stream(&stream);
let rate = prices.rate("USD", "CNY", date);              // Option<BigDecimal>
let value = prices.convert(&posting_units, "CNY", date); // Option<Amount>
```

It gives the same rates as Zhang's query engine uses for `convert`, `value` and `getprice`, so a plugin's valuations agree with Zhang's. The rules are beancount's:

- The rate on a date is the **latest `price` on or before it**. There is no rate before the first price.
- Prices of a pair on **the same day replace each other**: the last one in the stream wins.
- A pair without prices of its own uses the **inverse** of the opposite pair, `1 / rate`; zero prices have no inverse and are skipped.
- A pair quoted **in both directions** has one merged history. The direction with more prices is kept and the other one is inverted into it, so the rate is the latest quote in either direction. Of two quotes on the same day, the one of the less-quoted direction wins.
- A commodity's rate to itself is 1.

**Precision:** rates from `price` directives are exact. An inverse that terminates is exact too (`1 / 8 = 0.125`); one that does not is rounded half-even to 28 significant digits, as beancount's decimal context and Zhang's query engine do (`1 / 7 = 0.1428571428571428571428571429`). `convert` rounds a product to 28 significant digits only when it has more.

**Implicit prices:** `PriceMap::from_stream_with_implicit` also takes the prices written on postings, `@` per unit and `@@` in total, like beancount's `implicit_prices` plugin. Zhang itself does not use them, so the rates they add differ from what Zhang shows; it is an opt-in for plugins ported from beancount. A posting written without units is skipped: plugins see transactions before booking, so it carries no price yet.

## Reporting errors

There are two ways for a plugin to say something is wrong:

- **Report it** with `zhang_emit_error` (`errors::emit_error` / `emit_error_at`). The problem becomes a [`PluginError`](/user-guide/error-code/#pluginerror) in the ledger's error list, on the directive whose span you pass (or the plugin's directive), with the metas `plugin`, `message` and your own. The ledger still loads. This is how validators work.
- **Fail** the call: return an error from the handler (or trap, or panic). A failing processor or mapper aborts the whole load, and a call running past its `timeout` does too. Use it for problems the user must fix before the ledger means anything, such as invalid plugin config.

## Router plugins

A router plugin serves `/api/plugins/{name}` and every path below it, for any HTTP method:

```rust
use zhang_plugin_sdk::router::{self, Request, Response};
use zhang_plugin_sdk::{plugin, Error};

plugin! {
    name: "summary",
    version: env!("CARGO_PKG_VERSION"),
    router: route,
}

fn route(request: Request) -> Result<Response, Error> {
    match request.path.as_str() {
        "/balances" => Response::json(&router::query("SELECT account, sum(position) AS balance GROUP BY account")?),
        _ => Ok(Response::text("not found").with_status(404)),
    }
}
```

- **Route:** `{name}` is the plugin's name; the request's `path` is the part below the route.
- **Authentication:** the routes sit behind Zhang's own sign-in, and the credential headers never reach the plugin.
- **Read-only:** a router reads the ledger only through `zhang_query` (BQL) and `zhang_ledger_info`; no host function changes it. Every request runs in a fresh instance, and the ledger does not reload while it runs.
- **Security:** a router's pages are served from Zhang's own address, so a script on such a page can call Zhang's API, including the endpoints that change your ledger files, with your session. Escape everything you put into HTML.

[Router Plugins](/user-guide/router-plugins/) has the request and response JSON, the error statuses and the details.

## ABI v1 reference

This is what crosses the boundary, for plugins written without the Rust SDK. Every change is additive: a field is never removed or made required, so a plugin compiled for an older Zhang keeps working.

### Exports

Inputs and outputs are Extism plug-in input and output, as JSON.

| Export | Input | Output |
|---|---|---|
| `name` | none | the plugin's name, a JSON string: `"guard"` |
| `version` | none | its version, a JSON string: `"0.1.0"` |
| `supported_type` | none | a JSON array of `"Processor"`, `"Mapper"`, `"Router"` |
| `processor` | the stream: an array of directives | the new stream |
| `mapper` | one directive | an array of directives |
| `router` | the request | the response |

A directive is the serde JSON of `zhang-ast`'s `Spanned<Directive>`, of a kind ABI v1 knows: Zhang never hands a plugin a `pad` directive (see [the stage order contract](#the-stage-order-contract)). For example:

```json
{"data": {"Comment": {"content": "; a note"}}, "span": {"start": 0, "end": 8, "content": "; a note", "filename": "/ledger/main.zhang"}}
```

An export fails by returning a non-zero code with an Extism error; for a processor or mapper that aborts the load.

### Config

Zhang hands every plugin instance a string-to-string Extism config:

| Key | Value |
|---|---|
| a ledger option's key | the option's value |
| a meta key of the `plugin` directive | its value, the last one of a repeated key; wins over an option of the same key. `allowed_hosts` is never passed this way |
| `zhang.abi` | the ABI version, `"1"` |
| `zhang.plugin` | the directive as written, as JSON: `{"module": "…", "args": ["…"], "meta": {"key": ["value", …]}}`, with every value of every meta key (`allowed_hosts` included) |
| `zhang.seed` | the plugin's seed, a decimal `u64` |

A Zhang that sets no `zhang.abi` predates ABI v1: it sets none of the `zhang.*` keys and links none of the host functions below.

### Host functions

They live in the `extism:host/user` namespace. Arguments and results are `i64` offsets of Extism memory blocks. Every function returning a value answers JSON, `{"Ok": value}` or `{"Err": {"kind": "…", "message": "…"}}`, and none of them traps.

| Function | Argument | `Ok` value | Error kinds |
|---|---|---|---|
| `zhang_emit_error` | `{"message": "…", "span": {…}?, "metas": {"k": "v"}?}` | no result | none: an unreadable payload is itself reported |
| `zhang_now` | none | `{"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}` | none today; handle `Err` anyway |
| `zhang_read_file` | the path, UTF-8 | `{"content": "…", "encoding": "utf8" \| "base64"}` | `denied`, `not_found`, `too_large`, `unsupported`, `invalid` |
| `zhang_list_dir` | the path, UTF-8 | `{"entries": [{"name": "…", "kind": "file" \| "dir"}]}`, sorted by name | as `zhang_read_file` |
| `zhang_query` | the BQL text | `{"columns": [{"name": "…", "type": "…"}], "rows": [[…]]}` | `query` (with `line` and `column`), `invalid_input`, `unavailable` |
| `zhang_ledger_info` | none | `{"title": "…" \| null, "operating_currency": "CNY", "timezone": "Asia/Shanghai"}` | `unavailable` |

Where they work:

- `zhang_emit_error` reports into the error list from a processor or mapper. While the plugin registers (`name`, `version`, `supported_type`) what it reports is dropped, and in a router it is only logged.
- `zhang_now` works everywhere. In a router it reads the clock afresh for each request, and a call while registering does not make the ledger depend on the date.
- `zhang_read_file` and `zhang_list_dir` answer `denied` outside a processor or mapper, and for any path `allowed_paths` does not grant.
- `zhang_query` and `zhang_ledger_info` answer `unavailable` outside a router request.

### Minimum Zhang version

**Importing a host function makes a plugin fail to load on a Zhang that does not have it**, with an `unknown import` error, even if the plugin never calls it. The Rust SDK imports only the host functions your plugin actually calls.

| Feature | Zhang |
|---|---|
| `processor` and `mapper` exports, flat config (options and meta), `allowed_hosts` | 0.2.0 |
| `router` exports actually served, `zhang.abi`, `zhang.plugin`, `zhang.seed`, every host function above, `allowed_paths`, `timeout`, `seed`, `features.plugins`, unknown plugin types ignored | the release after 0.2.0 |
