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
    // the first PIVOT BY column labels the rows, the values of the second one become columns: one per year
    title: 'query.example.expenses_pivot',
    query: "SELECT year, root(account, 2) AS category, sum(cost(position)) AS total WHERE account ~ '^Expenses' GROUP BY 1, 2 PIVOT BY category, year",
  },
  {
    title: 'query.example.spending_by_payee',
    query: 'SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20',
  },
  {
    // an inventory can't be compared with a number, so HAVING sums the numbers of the costs
    title: 'query.example.categories_over_1000',
    query:
      "SELECT root(account, 2) AS category, sum(cost(position)) AS total WHERE account ~ '^Expenses' GROUP BY 1 HAVING sum(number(cost(position))) > 1000 ORDER BY total DESC",
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
  {
    title: 'query.example.latest_prices',
    query: 'SELECT * FROM #prices ORDER BY date DESC LIMIT 20',
  },
  {
    title: 'query.example.budgets',
    query: 'SELECT * FROM #budgets',
  },
  {
    title: 'query.example.ledger_errors',
    query: 'SELECT * FROM #errors',
  },
];

export const DEFAULT_QUERY = 'SELECT * ORDER BY date DESC LIMIT 50';
