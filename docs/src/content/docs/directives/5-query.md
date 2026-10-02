---
title: Query
description: Save named queries in your ledger with the query directive.
---

# Query Directives

The `query` directive saves a named query in your ledger, the same way Beancount and Fava do. Zhang lists saved queries in the **Saved** menu of the Query page, so you can run them again without retyping them (see [Saved queries](/user-guide/query-language/#saved-queries)). The query itself is written in Zhang's [query language](/user-guide/query-language/).

## Basic Syntax

```zhang
{DATE} query "{NAME}" "{QUERY}"
```

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"
```

- The query text must be quoted. It can span several lines.
- Metadata lines are allowed below the directive, as for any other dated directive.
- The same syntax works in Beancount files (`.bean`), so ledgers written for Fava keep their saved queries.

```zhang
2024-02-01 query "monthly food" "
  SELECT year, month, sum(position)
  WHERE account ~ '^Expenses:Food'
  GROUP BY year, month"
  owner: "alice"
```

## Escaping

The query text is a quoted string, like any other string in the ledger file, so a backslash in it starts an escape sequence:

- Write each backslash twice. To save the regular expression `\d+`, write `'\\d+'`:

  ```zhang
  2024-01-01 query "numbered" "SELECT narration WHERE narration ~ '\\d+'"
  ```

- Write a double quote as `\"`, or use single quotes for the strings inside the query, as in the examples on this page.

A backslash followed by a character that is not a known escape, such as `'\d+'` written with a single backslash, is an error that stops the ledger from loading ([#442](https://github.com/zhang-accounting/zhang/issues/442)). Beancount reads the doubled form the same way, so a ledger written like this works in both.

## Behaviour

- **Saved queries are not checked when the ledger is loaded.** A query that does not compile, for example one written for a feature Zhang does not support yet, is still saved and never makes the ledger report an error.
- **Every `query` directive is kept, including ones that share a name.** Queries are listed in ledger order: by date, then in the order they appear in your files. Use the date to tell queries with the same name apart.

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

`valid` tells whether the query compiles with the current query engine. When it does not, `error` holds the reason, with the line and column when they are known. Run a saved query by sending its `query` text to `POST /api/query`.
