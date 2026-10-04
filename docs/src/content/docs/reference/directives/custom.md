---
title: Custom
description: Reference for the custom directive, which carries dated values for plugins and tools.
sidebar:
  order: 9
---

A `custom` directive holds dated values that Zhang itself does not interpret. Plugins and other tools read them, for
example to take a setting that changes over time.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] custom "<Type>" <Value> [<Value> …]
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date the values apply from, optionally with a time of day. |
| `"<Type>"` | yes | What the directive is about. By convention, the name of the plugin that reads it. |
| `<Value>` | at least one | Values separated by spaces. Each is a quoted string, an account name, or a single word without spaces, quotes, colons, parentheses or commas, such as `100`, `CNY`, `2024-07-01` or `TRUE`. |

Metadata lines can follow the directive.

## Examples

```zhang
2024-01-01 custom "large-expense" "threshold" 100 CNY
2024-07-01 custom "large-expense" "threshold" "150 CNY"
2024-01-01 custom "reconcile" Assets:Bank:Checking "monthly"
```

## Behavior

- Zhang stores `custom` directives and checks nothing in them: an account named in a value does not have to exist.
- Every value is text. `100 CNY` is the two values `"100"` and `"CNY"`; reading them as an amount is up to the
  reader.
- The web UI does not show `custom` directives. They are rows of `#entries` in queries, with the type `custom`, and
  plugins receive them with the rest of the ledger.

### Plugin settings over time

Plugins built with the Rust SDK read their settings from `custom` directives written as

```text
YYYY-MM-DD custom "<plugin name>" "<key>" <Value> …
```

where `<plugin name>` is the name the plugin reports. A setting applies from its date: with the example above, an
entry dated 2024-03-05 sees the threshold `100 CNY`, one dated 2024-08-01 the threshold `150 CNY`. Of several
directives for one key on the same day, the last one wins. A setting in the entry's own metadata wins over the
`custom` directives, and those win over the metadata of the `plugin` directive and over options. Only a processor
plugin sees the whole ledger, so only a processor reads `custom` settings. See
[Config in `custom` directives](/developers/writing-plugins/#config-in-custom-directives).

## Errors

A `custom` directive produces no error. A plugin that reads it may report a
[`PluginError`](/reference/error-codes/#pluginerror).

## Beancount compatibility

- Beancount requires the type to be a quoted string, and its values to be quoted strings, numbers, amounts, dates,
  booleans or accounts. A bare word such as `CNY` alone, or `monthly` without quotes, is a syntax error there. Zhang
  reads both forms; write values in Beancount's form if the file must also load in Beancount or Fava.
- In a Beancount file, Zhang reads `custom budget …`, `custom budget-add …`, `custom budget-transfer …` and
  `custom budget-close …`, written with the bare word, as [budget directives](/reference/directives/budget/). Any other
  `custom` directive, including `custom "budget" …`, stays a `custom` directive.

## Related

- [Writing Plugins](/developers/writing-plugins/): reading `custom` directives from a plugin.
- [Plugin](/reference/directives/plugin/): declaring a plugin.
