// The budget pages' reads: the built-in queries `budgets.*` (POST /api/query/builtins), mapped by budget-rows.ts. A page
// passes the same query name and parameters to `OpenInExplore`, so what it shows and what opens on the Query page agree.
import { runBuiltin, runBuiltins } from '@/api/requests';
import { ledgerDate } from '@/components/query/explore-link';
import { type BudgetEvent, type BudgetInfo, type BudgetListItem, budgetEvents, budgetInfo, budgetListItem } from './budget-rows';

/** The `month` parameter of the `budgets.*` queries for the month of `date`: its first day, as a ledger date. */
export function budgetMonth(date: Date): string {
  return ledgerDate(new Date(date.getFullYear(), date.getMonth(), 1));
}

/** Every budget as of the month of `date`, by name (`budgets.month`). */
export async function retrieveBudgets(date: Date): Promise<BudgetListItem[]> {
  return (await runBuiltin('budgets.month', { month: budgetMonth(date) })).map(budgetListItem);
}

/** One budget as of the month of `date` (`budgets.budget` and `budgets.budget_month`, under one read of the ledger). */
export async function retrieveBudgetInfo(name: string, date: Date): Promise<BudgetInfo> {
  const [budget, month] = await runBuiltins([
    { name: 'budgets.budget', params: { name } },
    { name: 'budgets.budget_month', params: { name, month: budgetMonth(date) } },
  ]);
  const info = budgetInfo(budget[0], month[0]);
  if (!info) throw new Error(`there is no budget ${name}`);
  return info;
}

/**
 * What happened to a budget in the month of `date`, newest first (`budgets.events` and `budgets.postings`, under one read
 * of the ledger).
 */
export async function retrieveBudgetEvents(name: string, date: Date): Promise<BudgetEvent[]> {
  const month = budgetMonth(date);
  const [budget, events, postings] = await runBuiltins([
    { name: 'budgets.budget', params: { name } },
    { name: 'budgets.events', params: { name, month } },
    { name: 'budgets.postings', params: { name, month } },
  ]);
  if (budget.length === 0) throw new Error(`there is no budget ${name}`);
  return budgetEvents(events, postings);
}
