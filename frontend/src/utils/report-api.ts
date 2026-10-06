// The report's reads: the built-in queries `report.*` (POST /api/query/builtins), each page's figures under one read of
// the ledger, mapped by report.ts. A page passes the same query names and parameters to `OpenInExplore`.
import { runBuiltins } from '@/api/requests';
import { type ReportRank, type ReportSummary, reportRank, reportSummary } from './report';

/** A range of ledger dates, `YYYY-MM-DD`, both included. */
export interface LedgerDateRange {
  from: string;
  to: string;
}

/**
 * The summary of a range, every figure valued in `currency`, the operating currency: `report.net_worth` and
 * `report.liabilities` at the range's end, `report.flows` and `report.transaction_count` over it.
 */
export async function retrieveReportSummary({ from, to }: LedgerDateRange, currency: string): Promise<ReportSummary> {
  const [netWorth, liabilities, flows, count] = await runBuiltins([
    { name: 'report.net_worth', params: { to, currency } },
    { name: 'report.liabilities', params: { to, currency } },
    { name: 'report.flows', params: { from, to, currency } },
    { name: 'report.transaction_count', params: { from, to } },
  ]);
  return reportSummary(netWorth, liabilities, flows, count, currency);
}

/** The rank of an account `type` in a range: `report.account_totals` and `report.top_postings`, valued in `currency`. */
export async function retrieveReportRank(type: string, { from, to }: LedgerDateRange, currency: string): Promise<ReportRank> {
  const [totals, top] = await runBuiltins([
    { name: 'report.account_totals', params: { type, from, to, currency } },
    { name: 'report.top_postings', params: { type, from, to, currency } },
  ]);
  return reportRank(totals, top, currency);
}
