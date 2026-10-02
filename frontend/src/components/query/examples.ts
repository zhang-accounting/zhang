export interface QueryExample {
  /** i18n key of the example title */
  title: string;
  query: string;
}

export const QUERY_EXAMPLES: QueryExample[] = [
  {
    title: 'query.example.monthly_expenses',
    query: 'SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3',
  },
  {
    title: 'query.example.spending_by_payee',
    query: 'SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20',
  },
  {
    title: 'query.example.postings_with_tag',
    query: "SELECT date, payee, account, position WHERE 'trip' IN tags",
  },
  {
    title: 'query.example.holdings',
    query:
      'SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, convert(units(sum(position)), "USD") AS market WHERE account ~ "^Assets" GROUP BY account ORDER BY account',
  },
  {
    title: 'query.example.income_statement',
    query: "SELECT account, sum(position) FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ '^(Income|Expenses)' GROUP BY 1 ORDER BY 1",
  },
  {
    title: 'query.example.balances',
    query: 'BALANCES FROM CLOSE ON 2017-01-01 CLEAR',
  },
  {
    title: 'query.example.journal',
    query: "JOURNAL 'Assets:US:BofA:Checking'",
  },
  {
    title: 'query.example.recent_postings',
    query: 'SELECT * ORDER BY date DESC LIMIT 50',
  },
];

export const DEFAULT_QUERY = 'SELECT * ORDER BY date DESC LIMIT 50';
