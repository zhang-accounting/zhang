---
title: Query
description: Reference for the query directive, which saves a named query in the ledger.
sidebar:
  order: 13
---

The `query` directive saves a named query in your ledger, the same way Beancount and Fava do. Zhang lists saved
queries in the **Saved** menu of the Query page, so you can run them again without retyping them (see
[Saved queries](/reference/query-language/#saved-queries)). The query itself is written in Zhang's
[query language](/reference/query-language/).

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] query "<Name>" "<Query>"
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date of the query. It orders the list and tells queries with the same name apart. |
| `"<Name>"` | yes | The name shown in the **Saved** menu. A single word can be written without quotes. |
| `"<Query>"` | yes | The query text, in double quotes. It can span several lines. |

Metadata lines can follow the directive.

## Examples

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"

2024-02-01 query "monthly food" "
  SELECT year, month, sum(position)
  WHERE account ~ '^Expenses:Food'
  GROUP BY year, month"
  owner: "alice"
```

## Escaping

The query text is a quoted string, like any other string in the ledger file, so a backslash in it can start an escape
sequence:

- `\"` is a double quote and `\\` is a backslash. To use a double quote in the query, write `\"`, or use single quotes
  for the strings inside the query, as in the examples on this page.
- `\n`, `\t`, `\r`, `\b`, `\f`, `\/` and `\uXXXX` are read as in JSON. The escapes that older versions of Zhang wrote
  are still read as the character they stand for: `\$`, `` \` ``, `\a`, `\v`, `\e` and `\u{...}`, such as `\u{a0}`.
- A backslash followed by any other character is kept as written. So the regular expression `\d+` can be written as
  it is:

  ```zhang
  2024-01-01 query "numbered" "SELECT narration WHERE narration ~ '\d+'"
  ```

- Writing the backslash twice, as in `'\\d+'`, also works and reads the same. Write it twice when the backslash comes
  before one of the characters listed above. In a regular expression, `'\\b'` is a word boundary, but `'\b'` is read
  as a backspace character; `'\\$'` matches a dollar sign, but `'\$'` is read as `$`, the end of the text.

A malformed `\u` escape, such as `\uZZZZ`, is an error that stops the ledger from loading. The error gives the line
and column of the escape.

## Behavior

- **Saved queries are not checked when the ledger is loaded.** A query that does not compile, for example one
  written for a feature Zhang does not support yet, is still saved and never makes the ledger report an error. The
  **Saved** menu and the HTTP API say whether each query compiles.
- **Every `query` directive is kept, including ones that share a name.** Queries are listed in ledger order: by date,
  then in the order they appear in your files.

## HTTP API

`GET /api/query/saved` lists the saved queries:

```json
{
  "data": [
    {
      "name": "food by payee",
      "query": "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee",
      "date": "2024-01-01",
      "valid": true,
      "error": null
    }
  ]
}
```

`valid` tells whether the query compiles with the current query engine. When it does not, `error` holds the reason,
with the line and column when they are known. Run a saved query by sending its `query` text to `POST /api/query`.

## Errors

A `query` directive produces no ledger error.

## Beancount compatibility

The same syntax works in Beancount files (`.bean`), so ledgers written for Fava keep their saved queries. Zhang reads
their strings with the escaping rules above. Beancount itself drops a backslash that does not start an escape it
knows, so it reads `'\d+'` as `'d+'`. It has no `\uXXXX`, `\a`, `\v` or `\e` escape either, so it reads `é` as
`u00e9` where Zhang reads `é`. If you also use the ledger with Beancount or Fava, write each backslash twice, as in
`'\\d+'`. That form reads the same in both.

For how Zhang's query language differs from Beancount's, see
[Differences from BQL and beanquery](/reference/query-language/#differences-from-bql-and-beanquery).

## Related

- [Querying](/guides/querying/): the Query page and common queries.
- [Query language](/reference/query-language/): the full reference.
