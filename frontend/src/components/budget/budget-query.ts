// Type-only imports keep this module runnable by `node --test` (budget-query.test.ts).
import type { BudgetInfo } from '@/api/types';
import type { BuiltinParamInput } from '../query/explore-link.ts';

/**
 * The parameters of the built-in query `budgets.postings` for a budget's activity in a month (its first day), as
 * `GET /api/budgets/{name}/interval/{year}/{month}` binds them: the budget's accounts and name, and its close (date and
 * time), after which nothing counts toward it. The close comes from the budget info; `null` while it is open.
 */
export function budgetPostingsParams(
  budget: Pick<BudgetInfo, 'name' | 'related_accounts' | 'close' | 'close_time'>,
  month: Date,
): Record<string, BuiltinParamInput> {
  return {
    accounts: budget.related_accounts,
    month,
    name: budget.name,
    close: budget.close ?? null,
    close_time: budget.close_time ?? null,
  };
}
