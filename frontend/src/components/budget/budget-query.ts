// Type-only imports keep this module runnable by `node --test` (budget-query.test.ts).
import type { BudgetInfo } from '@/api/types';
import type { BuiltinParamInput } from '../query/explore-link.ts';

/**
 * The parameters of the built-in query `budgets.postings` for a budget's activity in a month (its first day), as
 * `GET /api/budgets/{name}/interval/{year}/{month}` binds them: the budget's name and the month. The query lists the
 * postings that count toward the budget (its `budgets` column), so it needs neither the budget's accounts nor its close.
 */
export function budgetPostingsParams(budget: Pick<BudgetInfo, 'name'>, month: Date): Record<string, BuiltinParamInput> {
  return {
    month,
    name: budget.name,
  };
}
