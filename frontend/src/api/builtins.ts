// Generated from the built-in queries (zhang-server/src/builtin.rs) by their tests; do not edit.
// Regenerate with `ZHANG_WRITE_BUILTINS_TS=1 cargo test -p zhang-server the_frontend_types_of_the_builtins_are_up_to_date`.
import type { QueryAmount, QueryInventory, QueryMeta, QueryPosition } from './types';

/**
 * Every built-in query (`GET /api/query/builtins`): the `params` that `POST /api/query/builtins/{name}` takes, and
 * the `row` it returns, by column name (`runBuiltin` in requests.ts). Any cell may be `null`.
 */
export interface Builtins {
  'postings.between': {
    params: {
      from: string | null;
      to: string | null;
    };
    row: {
      date: string | null;
      flag: string | null;
      payee: string | null;
      narration: string | null;
      account: string | null;
      position: QueryPosition | null;
    };
  };
  'postings.matching': {
    params: {
      payee: string | null;
      tags: string[] | null;
    };
    row: {
      date: string | null;
      payee: string | null;
      narration: string | null;
      tags: string[] | null;
      account: string | null;
      position: QueryPosition | null;
    };
  };
  'report.net_worth': {
    params: {
      to: string | null;
      currency: string | null;
    };
    row: {
      balance: QueryInventory | null;
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.liabilities': {
    params: {
      to: string | null;
      currency: string | null;
    };
    row: {
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.flows': {
    params: {
      from: string | null;
      to: string | null;
      currency: string | null;
    };
    row: {
      type: string | null;
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.transaction_count': {
    params: {
      from: string | null;
      to: string | null;
    };
    row: {
      transactions: number | null;
    };
  };
  'report.net_worth_trend': {
    params: {
      from: string | null;
      to: string | null;
      interval: string | null;
      currency: string | null;
    };
    row: {
      bucket: string | null;
      balance: QueryInventory | null;
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.changes': {
    params: {
      from: string | null;
      to: string | null;
      interval: string | null;
      currency: string | null;
    };
    row: {
      bucket: string | null;
      type: string | null;
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.account_totals': {
    params: {
      type: string | null;
      from: string | null;
      to: string | null;
      currency: string | null;
    };
    row: {
      account: string | null;
      units: QueryInventory | null;
      value: QueryInventory | null;
    };
  };
  'report.top_postings': {
    params: {
      type: string | null;
      from: string | null;
      to: string | null;
      currency: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      account: string | null;
      id: string | null;
      payee: string | null;
      narration: string | null;
      units: QueryAmount | null;
      account_balance: QueryAmount | null;
      value: QueryAmount | null;
    };
  };
  'accounts.list': {
    params: {
      date: string | null;
      time: string | null;
    };
    row: {
      account: string | null;
      open: string | null;
      close: string | null;
      alias: string | null;
      status: string | null;
    };
  };
  'accounts.balances': {
    params: {
      operating_currency: string | null;
    };
    row: {
      account: string | null;
      currency: string | null;
      units: string | null;
      value: QueryInventory | null;
      first_date: string | null;
    };
  };
  'accounts.subtree': {
    params: {
      account: string | null;
      date: string | null;
      time: string | null;
    };
    row: {
      account: string | null;
      open: string | null;
      close: string | null;
      alias: string | null;
      status: string | null;
    };
  };
  'accounts.subtree_balances': {
    params: {
      account: string | null;
      operating_currency: string | null;
    };
    row: {
      account: string | null;
      currency: string | null;
      units: string | null;
      value: QueryInventory | null;
      first_date: string | null;
    };
  };
  'accounts.journal': {
    params: {
      account: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      flag: string | null;
      id: string | null;
      account: string | null;
      payee: string | null;
      narration: string | null;
      seq: number | null;
      posting_index: number | null;
      units: string | null;
      currency: string | null;
      balance: QueryInventory | null;
    };
  };
  'accounts.journal_page': {
    params: {
      account: string | null;
      limit: number | null;
      offset: number | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      flag: string | null;
      id: string | null;
      account: string | null;
      payee: string | null;
      narration: string | null;
      seq: number | null;
      posting_index: number | null;
      units: string | null;
      currency: string | null;
      balance: QueryInventory | null;
    };
  };
  'accounts.balance_assertions': {
    params: {
      account: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      id: string | null;
      account: string | null;
      amount: QueryAmount | null;
      tolerance: string | null;
      actual: QueryAmount | null;
      difference: QueryAmount | null;
      passed: boolean | null;
      pad: string | null;
      seq: number | null;
    };
  };
  'accounts.balance_history': {
    params: {
      account: string | null;
    };
    row: {
      date: string | null;
      currency: string | null;
      balance: QueryAmount | null;
    };
  };
  'accounts.documents': {
    params: {
      account: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      account: string | null;
      path: string | null;
      transaction_id: string | null;
    };
  };
  'journals.page': {
    params: {
      keyword: string | null;
      tags: string[] | null;
      links: string[] | null;
      size: number | null;
      offset: number | null;
    };
    row: {
      seq: number | null;
      type: string | null;
      id: string | null;
      date: string | null;
      time: string | null;
      flag: string | null;
      payee: string | null;
      narration: string | null;
      tags: string[] | null;
      links: string[] | null;
      metas: QueryMeta[] | null;
    };
  };
  'journals.postings': {
    params: {
      ids: string[] | null;
    };
    row: {
      id: string | null;
      posting_index: number | null;
      account: string | null;
      automatic: boolean | null;
      balanced: boolean | null;
      currency: string | null;
      number: string | null;
      lots: number | null;
      lots_at_cost: number | null;
      cost_number: string | null;
      max_cost_number: string | null;
      cost_currency: string | null;
      max_cost_currency: string | null;
      balance_before: string | null;
      balance_after: string | null;
      metas: QueryMeta[] | null;
    };
  };
  'journals.balance_checks': {
    params: {
      ids: string[] | null;
    };
    row: {
      id: string | null;
      account: string | null;
      amount: QueryAmount | null;
      tolerance: string | null;
      actual: QueryAmount | null;
      difference: QueryAmount | null;
      passed: boolean | null;
    };
  };
  'journals.payees': {
    params: Record<string, never>;
    row: {
      payee: string | null;
    };
  };
  'journals.accounts': {
    params: {
      date: string | null;
      time: string | null;
    };
    row: {
      account: string | null;
    };
  };
  'journals.documents': {
    params: Record<string, never>;
    row: {
      date: string | null;
      time: string | null;
      path: string | null;
      account: string | null;
      transaction_id: string | null;
    };
  };
  'journals.errors': {
    params: {
      size: number | null;
      offset: number | null;
    };
    row: {
      id: string | null;
      kind: string | null;
      file: string | null;
      line: number | null;
      column: number | null;
      span_start: number | null;
      span_end: number | null;
      source: string | null;
      metas: QueryMeta[] | null;
    };
  };
  'budgets.month': {
    params: {
      month: string | null;
    };
    row: {
      name: string | null;
      alias: string | null;
      category: string | null;
      currency: string | null;
      last_month: string | null;
      assigned: QueryAmount | null;
      activity: string | null;
      available: QueryAmount | null;
      closed: boolean | null;
    };
  };
  'budgets.budget': {
    params: {
      name: string | null;
    };
    row: {
      name: string | null;
      alias: string | null;
      category: string | null;
      currency: string | null;
      accounts: string[] | null;
      close: string | null;
      close_time: string | null;
    };
  };
  'budgets.budget_month': {
    params: {
      name: string | null;
      month: string | null;
    };
    row: {
      name: string | null;
      alias: string | null;
      category: string | null;
      currency: string | null;
      last_month: string | null;
      assigned: QueryAmount | null;
      activity: string | null;
      available: QueryAmount | null;
      closed: boolean | null;
    };
  };
  'budgets.events': {
    params: {
      name: string | null;
      month: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      type: string | null;
      amount: QueryAmount | null;
    };
  };
  'budgets.postings': {
    params: {
      name: string | null;
      month: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      timestamp: number | null;
      account: string | null;
      id: string | null;
      payee: string | null;
      narration: string | null;
      units: QueryAmount | null;
      balance: QueryAmount | null;
    };
  };
  'commodities.totals': {
    params: Record<string, never>;
    row: {
      currency: string | null;
      total: string | null;
    };
  };
  'commodities.total': {
    params: {
      commodity: string | null;
    };
    row: {
      currency: string | null;
      total: string | null;
    };
  };
  'commodities.latest_prices': {
    params: {
      currency: string | null;
    };
    row: {
      currency: string | null;
      date: string | null;
      time: string | null;
      rate: string | null;
    };
  };
  'commodities.latest_price': {
    params: {
      commodity: string | null;
      currency: string | null;
    };
    row: {
      currency: string | null;
      date: string | null;
      time: string | null;
      rate: string | null;
    };
  };
  'commodities.lots': {
    params: {
      commodity: string | null;
    };
    row: {
      account: string | null;
      cost_date: string | null;
      cost_number: string | null;
      cost_currency: string | null;
      cost_label: string | null;
      units: string | null;
    };
  };
  'commodities.prices': {
    params: {
      commodity: string | null;
    };
    row: {
      date: string | null;
      time: string | null;
      amount: QueryAmount | null;
    };
  };
}
