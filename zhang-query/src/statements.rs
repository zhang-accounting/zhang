//! The `BALANCES` and `JOURNAL` statements, desugared into a [`Select`] the way beanquery
//! defines them, so that compiling, optimising and executing only ever see SELECT.
//!
//! ```text
//! BALANCES [AT f] [FROM x] [WHERE y]
//!   =  SELECT account, sum(f(position))
//!      FROM x WHERE y
//!      GROUP BY account, account_sortkey(account)
//!      ORDER BY account_sortkey(account)
//!
//! JOURNAL ['regex'] [AT f] [FROM x]
//!   =  SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account,
//!             f(position), f(balance)
//!      FROM x WHERE account ~ 'regex'
//! ```
//!
//! Without `AT`, `f(e)` is just `e`. `AT f` names any function with a one-argument overload
//! for positions (and, for JOURNAL, inventories): `units`, `cost` and `value`, typically.
//!
//! The result columns are named like the equivalent SELECT: `sum(position)`,
//! `sum(cost(position))`, `maxwidth(payee, 48)`, `cost(balance)`. (beanquery spells the
//! BALANCES column `SUM((position))` / `SUM(cost(position))`.)
//!
//! The synthesized expressions have no source text of their own: they get an empty span at
//! the statement keyword, except the `AT` function call, which spans the function name, and
//! the JOURNAL pattern, which keeps its own span, so errors point at what the user wrote.
//!
//! The FROM clause is not part of the desugaring: the parser reads it with the same function
//! as for SELECT and sets [`Select::from`] of the result, so every form of FROM SELECT
//! accepts works for BALANCES and JOURNAL too.

use crate::ast::{BinaryOp, Expr, ExprKind, Literal, OrderItem, Select, Target, Targets};
use crate::error::Span;

/// `AT name`: the function BALANCES and JOURNAL apply to positions and balances.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AtFunction {
    /// lower-cased
    pub name: String,
    pub span: Span,
}

/// The widths beanquery's JOURNAL truncates payees and narrations to.
const PAYEE_WIDTH: i64 = 48;
const NARRATION_WIDTH: i64 = 80;

/// Builds the expressions of one statement.
struct Synth {
    /// the empty span every synthesized node gets
    span: Span,
}

impl Synth {
    fn at(keyword: Span) -> Synth {
        Synth {
            span: Span::new(keyword.start, keyword.start),
        }
    }

    fn column(&self, name: &str) -> Expr {
        Expr::new(ExprKind::Column(name.to_owned()), self.span)
    }

    fn call(&self, name: &str, args: Vec<Expr>, span: Span) -> Expr {
        Expr::new(
            ExprKind::Call {
                name: name.to_owned(),
                args,
                star: false,
            },
            span,
        )
    }

    /// `f(column)` for `AT f`, else the column; with its result column name.
    fn summarized(&self, at: Option<&AtFunction>, column: &str) -> (Expr, String) {
        match at {
            Some(at) => {
                let arg = Expr::new(ExprKind::Column(column.to_owned()), at.span);
                (self.call(&at.name, vec![arg], at.span), format!("{}({})", at.name, column))
            }
            None => (self.column(column), column.to_owned()),
        }
    }

    fn account_sortkey(&self) -> Expr {
        self.call("account_sortkey", vec![self.column("account")], self.span)
    }

    /// `maxwidth(column, width)`
    fn maxwidth(&self, column: &str, width: i64) -> Target {
        let width_literal = Expr::new(ExprKind::Literal(Literal::Int(width)), self.span);
        Target {
            expr: self.call("maxwidth", vec![self.column(column), width_literal], self.span),
            alias: Some(format!("maxwidth({}, {})", column, width)),
        }
    }

    fn target(&self, name: &str) -> Target {
        Target {
            expr: self.column(name),
            alias: Some(name.to_owned()),
        }
    }
}

/// `BALANCES [AT f] [WHERE where_clause]`, without its FROM clause.
pub(crate) fn balances(keyword: Span, at: Option<AtFunction>, where_clause: Option<Expr>) -> Select {
    let synth = Synth::at(keyword);
    let (summarized, name) = synth.summarized(at.as_ref(), "position");
    // `sum` spans the AT function too: an AT function sum cannot add up is reported there
    let sum_span = at.as_ref().map_or(synth.span, |at| at.span);
    let sum = synth.call("sum", vec![summarized], sum_span);
    Select {
        distinct: false,
        targets: Targets::List(vec![
            synth.target("account"),
            Target {
                expr: sum,
                alias: Some(format!("sum({})", name)),
            },
        ]),
        table: None,
        from: None,
        period: None,
        where_clause,
        group_by: Some(vec![synth.column("account"), synth.account_sortkey()]),
        having: None,
        order_by: Some(vec![OrderItem {
            expr: synth.account_sortkey(),
            descending: false,
        }]),
        pivot_by: None,
        limit: None,
    }
}

/// `JOURNAL [account] [AT f]`, without its FROM clause; `account` is a regular expression (a
/// string literal or a parameter) the account must match.
pub(crate) fn journal(keyword: Span, account: Option<Expr>, at: Option<AtFunction>) -> Select {
    let synth = Synth::at(keyword);
    let mut targets = vec![
        synth.target("date"),
        synth.target("flag"),
        synth.maxwidth("payee", PAYEE_WIDTH),
        synth.maxwidth("narration", NARRATION_WIDTH),
        synth.target("account"),
    ];
    for column in ["position", "balance"] {
        let (expr, name) = synth.summarized(at.as_ref(), column);
        targets.push(Target { expr, alias: Some(name) });
    }
    let where_clause = account.map(|pattern| {
        let span = Span::new(synth.span.start, pattern.span.end);
        Expr::new(ExprKind::Binary(BinaryOp::Match, Box::new(synth.column("account")), Box::new(pattern)), span)
    });
    Select {
        distinct: false,
        targets: Targets::List(targets),
        table: None,
        from: None,
        period: None,
        where_clause,
        group_by: None,
        having: None,
        order_by: None,
        pivot_by: None,
        limit: None,
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{Expr, Select, Targets};
    use crate::parser::parse;

    fn same_exprs(a: &[Expr], b: &[Expr]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_as(b))
    }

    fn same_option(a: &Option<Expr>, b: &Option<Expr>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.same_as(b),
            (a, b) => a.is_none() && b.is_none(),
        }
    }

    /// Whether two parsed queries are the same SELECT, ignoring source positions and aliases.
    /// The FROM clauses are only compared for presence: the parser reads them for SELECT and
    /// the statements alike (`tests/statements.rs` compares the results).
    fn same_select(a: &Select, b: &Select) -> bool {
        let targets = |select: &Select| match &select.targets {
            Targets::List(targets) => targets.iter().map(|it| it.expr.clone()).collect::<Vec<_>>(),
            Targets::Wildcard => panic!("a wildcard"),
        };
        let order = |select: &Select| select.order_by.iter().flatten().map(|it| (it.expr.clone(), it.descending)).collect::<Vec<_>>();
        let (a_order, b_order) = (order(a), order(b));
        a.distinct == b.distinct
            && same_exprs(&targets(a), &targets(b))
            && a.from.is_some() == b.from.is_some()
            && same_option(&a.where_clause, &b.where_clause)
            && same_exprs(a.group_by.as_deref().unwrap_or_default(), b.group_by.as_deref().unwrap_or_default())
            && a.group_by.is_some() == b.group_by.is_some()
            && a_order.len() == b_order.len()
            && a_order.iter().zip(&b_order).all(|((a, a_desc), (b, b_desc))| a.same_as(b) && a_desc == b_desc)
            && a.limit == b.limit
    }

    #[test]
    fn statements_desugar_into_the_select_beanquery_defines() {
        for (statement, select) in [
            (
                "BALANCES",
                "SELECT account, sum(position) GROUP BY account, account_sortkey(account) ORDER BY account_sortkey(account)",
            ),
            (
                "BALANCES AT cost FROM year = 2016 WHERE account ~ 'Assets'",
                "SELECT account, sum(cost(position)) FROM year = 2016 WHERE account ~ 'Assets' \
                 GROUP BY account, account_sortkey(account) ORDER BY account_sortkey(account)",
            ),
            (
                "JOURNAL",
                "SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account, position, balance",
            ),
            (
                "JOURNAL 'Cash' AT units FROM month = 1",
                "SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account, units(position), units(balance) \
                 FROM month = 1 WHERE account ~ 'Cash'",
            ),
            (
                "JOURNAL $1",
                "SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account, position, balance WHERE account ~ $1",
            ),
        ] {
            let desugared = parse(statement).unwrap();
            let expected = parse(select).unwrap();
            assert!(same_select(&desugared, &expected), "{}\n{:#?}", statement, desugared);
        }
    }
}
