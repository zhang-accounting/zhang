// The budget pages' figures from the rows of the built-in queries `budgets.*`, by the rules the server's budget endpoints
// followed before (#479 decision 8). Type-only imports keep this module runnable by `node --test` (budget-rows.test.ts).
import type { Builtins } from '@/api/builtins';
import type { QueryAmount } from '@/api/types';

type MonthRow = Builtins['budgets.month']['row'];
type BudgetRow = Builtins['budgets.budget']['row'];
type EventRow = Builtins['budgets.events']['row'];
type PostingRow = Builtins['budgets.postings']['row'];

/** An amount as the pages render it. */
export interface BudgetAmount {
  number: string;
  commodity: string;
}

/** A budget as of a month: a row of `budgets.month`. */
export interface BudgetListItem {
  name: string;
  alias: string | null;
  category: string | null;
  /** whether the budget was closed in or before the month */
  closed: boolean;
  assigned_amount: BudgetAmount;
  activity_amount: BudgetAmount;
  available_amount: BudgetAmount;
}

/** One budget as of a month: its definition (`budgets.budget`) and its figures in the month (`budgets.budget_month`). */
export interface BudgetInfo extends BudgetListItem {
  /** the date of the budget's close, whatever the month asked for; `null` if it is never closed */
  close: string | null;
  /** the time of day of the close (`HH:MM:SS`); `null` for a close without a time, or if it is never closed */
  close_time: string | null;
  /** the accounts whose postings are the budget's activity, by name */
  related_accounts: string[];
}

/** What a `budget-add` or `budget-transfer` directive put into the budget: a row of `budgets.events`. */
export interface BudgetEventItem {
  type: 'BudgetEvent';
  datetime: string;
  timestamp: number;
  amount: BudgetAmount;
  event_type: 'AddAssignedAmount' | 'Transfer';
}

/** A posting that counts toward the budget: a row of `budgets.postings`. */
export interface BudgetPostingItem {
  type: 'Posting';
  datetime: string;
  timestamp: number;
  account: string;
  trx_id: string;
  payee: string | null;
  narration: string | null;
  inferred_unit: BudgetAmount;
  /** the balance of the account in the posting's commodity right after it */
  account_after: BudgetAmount;
}

export type BudgetEvent = BudgetEventItem | BudgetPostingItem;

/** A query amount as the pages render it; `null` is zero in `commodity`. */
function amount(cell: QueryAmount | null, commodity: string): BudgetAmount {
  return cell ? { number: cell.number, commodity: cell.currency } : { number: '0', commodity };
}

/** The ledger's wall-clock date and time of a row, as the API wrote a `datetime`. */
function datetime(date: string | null, time: string | null): string {
  return `${date ?? ''}T${time ?? '00:00:00'}`;
}

/** A budget's figures in a month from a row of `budgets.month` or `budgets.budget_month`. */
function figures(row: MonthRow): Pick<BudgetListItem, 'closed' | 'assigned_amount' | 'activity_amount' | 'available_amount'> {
  const commodity = row.currency ?? '';
  return {
    closed: row.closed ?? false,
    assigned_amount: amount(row.assigned, commodity),
    // a number in the budget's currency, `0` in a month the query carries the budget over to
    activity_amount: { number: row.activity ?? '0', commodity },
    available_amount: amount(row.available, commodity),
  };
}

/** A budget of the month's list. */
export function budgetListItem(row: MonthRow): BudgetListItem {
  return { name: row.name ?? '', alias: row.alias, category: row.category, ...figures(row) };
}

/**
 * One budget as of a month: `null` when there is no such budget (`budget` is the row of `budgets.budget`, if any). Before
 * the budget's first month there is no row of `budgets.budget_month`: nothing is assigned or spent, and it is not closed.
 */
export function budgetInfo(budget: BudgetRow | undefined, month: MonthRow | undefined): BudgetInfo | null {
  if (!budget) return null;
  const commodity = budget.currency ?? '';
  const zero = { number: '0', commodity };
  return {
    name: budget.name ?? '',
    alias: budget.alias,
    category: budget.category,
    close: budget.close,
    close_time: budget.close_time,
    related_accounts: [...(budget.accounts ?? [])],
    ...(month ? figures(month) : { closed: false, assigned_amount: zero, activity_amount: zero, available_amount: zero }),
  };
}

/**
 * What happened to a budget in a month, newest first: its events (`budgets.events`) and the postings that count toward it
 * (`budgets.postings`), both newest first already; of the same time, the budget's own entries come first.
 */
export function budgetEvents(events: EventRow[], postings: PostingRow[]): BudgetEvent[] {
  const items: BudgetEvent[] = events.map((row) => ({
    type: 'BudgetEvent',
    datetime: datetime(row.date, row.time),
    timestamp: row.timestamp ?? 0,
    amount: amount(row.amount, ''),
    event_type: row.type === 'assign' ? 'AddAssignedAmount' : 'Transfer',
  }));
  const merged: BudgetEvent[] = [];
  let next = 0;
  for (const row of postings) {
    const units = amount(row.units, '');
    const posting: BudgetPostingItem = {
      type: 'Posting',
      datetime: datetime(row.date, row.time),
      timestamp: row.timestamp ?? 0,
      account: row.account ?? '',
      trx_id: row.id ?? '',
      payee: row.payee,
      narration: row.narration || null,
      inferred_unit: units,
      account_after: amount(row.balance, units.commodity),
    };
    while (next < items.length && items[next].timestamp >= posting.timestamp) merged.push(items[next++]);
    merged.push(posting);
  }
  return merged.concat(items.slice(next));
}
