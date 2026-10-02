---
title: Query
description: Save named queries in your ledger with the query directive.
---

# Query Directives

The `query` directive saves a named query in your ledger, the same way Beancount and Fava do. Zhang lists saved queries so you can run them again without retyping them. The query itself is written in Zhang's [query language](/user-guide/query-language/).

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
