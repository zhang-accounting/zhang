//! Label columns: a set column of `postings` whose value, for a row, is the labels whose predicate holds for it.
//!
//! A provider declares the predicate once, as BQL over a row of `postings` with parameters ([`super::LabelDecl`]); it
//! is compiled once with the catalog, against the core `postings` table, so it reads no other label column and no
//! table of a provider. The snapshot holds the labels, each with the parameters it binds ([`super::Label`]); `:label`
//! is the label itself. An execution binds the predicate to every label once ([`Labels`]), as it binds a query's own
//! parameters, and then evaluates it per row:
//!
//! - the column's value evaluates the predicates of the labels the row can have: when the predicate starts with
//!   `<column> IN :<parameter>` (after conjuncts that cannot fail), only those of the labels whose parameter holds the
//!   row's value in that column, found in an index built once per execution;
//! - `:x IN <column>` (and `'x' IN <column>`) evaluates the predicate of the label `x` only.
//!
//! A predicate that fails with an error on a row does not hold there.

use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use super::LabelDecl;
use crate::catalog::{Catalog, ColumnRef};
use crate::compiler::CExpr;
use crate::error::QueryErrorKind;
use crate::executor::{Env, RegexCache};
use crate::params::{ParamRef, ParamTypes, Params};
use crate::projector::infallible;
use crate::table::{Borrow, Dataset, Row, RowRef};
use crate::value::{DataType, Value};
use crate::Query;

/// A label column of a catalog: what its values are computed with.
pub(crate) struct LabelColumn {
    /// the provider whose snapshot holds the labels
    pub provider: Arc<str>,
    /// the column's name, under which the snapshot holds the labels
    pub name: String,
    /// the predicate, compiled and optimized against the core `postings` table, with its parameters
    predicate: CExpr,
    /// a conjunct `<column> IN :<parameter>` of the predicate, every conjunct before which cannot fail: the labels are
    /// indexed by the values of their parameter
    pub(super) index: Option<(ColumnRef, ParamRef)>,
}

/// What a query's text must be to compile the predicate as the filter of a query.
const PREFIX: &str = "SELECT count(*) WHERE ";

impl LabelColumn {
    /// The label column `decl` of `provider`, its predicate compiled: a row-level boolean expression of the core
    /// `postings` table that reads no running total.
    pub fn compile(provider: &Arc<str>, decl: &LabelDecl) -> Result<LabelColumn, String> {
        let mut types = ParamTypes::new().bind("label", DataType::Str);
        for (name, ty) in &decl.params {
            if name == "label" {
                return Err("a parameter is named label, which is the label".to_owned());
            }
            types = types.bind(name.as_str(), *ty);
        }
        let text = format!("{}{}", PREFIX, decl.predicate);
        let query = Query::compile_in(Catalog::core(), &text, &types).map_err(|err| {
            let at = match (err.line, err.column) {
                (Some(1), Some(column)) => format!(" (column {})", column.saturating_sub(PREFIX.len())),
                (Some(line), Some(column)) => format!(" (line {}, column {})", line, column),
                _ => String::new(),
            };
            let kind = if err.kind == QueryErrorKind::Parse {
                "does not parse"
            } else {
                "does not compile"
            };
            format!("its predicate {}: {}{}", kind, err.message, at)
        })?;
        let Some(predicate) = query.plan.filter.clone() else {
            return Err("its predicate always holds".to_owned());
        };
        if reads_running(&predicate) {
            return Err("its predicate reads a running balance, which depends on the rows a query selects".to_owned());
        }
        let index = index_of(&predicate);
        Ok(LabelColumn {
            provider: provider.clone(),
            name: decl.name.clone(),
            predicate,
            index,
        })
    }
}

fn reads_running(expr: &CExpr) -> bool {
    matches!(expr, CExpr::Running(_)) || expr.children().into_iter().any(reads_running)
}

/// The conjunct `<column> IN :<parameter>` of `predicate` that every row it holds for satisfies, and before which no
/// conjunct can fail with an error: rows whose value is in no label's parameter cannot hold.
fn index_of(predicate: &CExpr) -> Option<(ColumnRef, ParamRef)> {
    let conjuncts = match predicate {
        CExpr::And(operands) => operands.as_slice(),
        predicate => std::slice::from_ref(predicate),
    };
    for conjunct in conjuncts {
        if let CExpr::InSet { needle, set, negated: false } = conjunct {
            if let (CExpr::Column(column), CExpr::Param(param)) = (needle.as_ref(), set.as_ref()) {
                if column.ty() == DataType::Str {
                    return Some((column.clone(), param.clone()));
                }
            }
        }
        if !infallible(conjunct) {
            return None;
        }
    }
    None
}

/// A label column for one execution: the snapshot's labels, the predicate bound to each, and the index.
pub(crate) struct Labels<'a> {
    labels: &'a [super::Label],
    /// the column, whose predicate is bound to each label
    column: Arc<LabelColumn>,
    /// the predicate of every label, its parameters bound when the label is first tested
    bound: Vec<OnceCell<CExpr>>,
    /// the position of every label
    by_name: HashMap<&'a str, usize>,
    /// the column of the index, and for each of its values the positions of the labels whose parameter holds it
    index: Option<(ColumnRef, HashMap<&'a str, Vec<usize>>)>,
    regexes: RegexCache,
}

impl<'a> Labels<'a> {
    /// The labels `labels` of `column`, its predicate to be bound to each.
    pub fn resolve(column: &Arc<LabelColumn>, labels: &'a [super::Label]) -> Labels<'a> {
        let bound = labels.iter().map(|_| OnceCell::new()).collect();
        let by_name = labels.iter().enumerate().map(|(at, label)| (label.label.as_str(), at)).collect();
        let index = column.index.as_ref().map(|(indexed, param)| {
            let mut index: HashMap<&'a str, Vec<usize>> = HashMap::new();
            for (at, label) in labels.iter().enumerate() {
                if let Some(Value::Set(values)) = label.params.get(param) {
                    for value in values {
                        index.entry(value.as_str()).or_default().push(at);
                    }
                }
            }
            (indexed.clone(), index)
        });
        Labels {
            labels,
            column: column.clone(),
            bound,
            by_name,
            index,
            regexes: RegexCache::default(),
        }
    }

    /// The labels whose predicate holds for `row`: the column's value.
    pub fn of_row<'d>(&self, data: &Dataset<'d>, row: &Row<'d>) -> BTreeSet<String> {
        let holds = |at: &usize| self.holds_at(*at, data, row);
        let label = |at: &usize| self.labels[*at].label.clone();
        match &self.index {
            Some((column, index)) => match indexed_value(column, data, row) {
                Some(value) => index
                    .get(value.as_ref())
                    .map(|labels| labels.iter().filter(|at| holds(at)).map(label).collect()),
                None => None,
            }
            .unwrap_or_default(),
            None => (0..self.labels.len()).filter(holds).map(|at| label(&at)).collect(),
        }
    }

    /// Whether the predicate of the label `label` holds for `row`: `label IN <column>`.
    pub fn holds<'d>(&self, data: &Dataset<'d>, row: &Row<'d>, label: &str) -> bool {
        self.by_name.get(label).is_some_and(|at| self.holds_at(*at, data, row))
    }

    fn holds_at<'d>(&self, at: usize, data: &Dataset<'d>, row: &Row<'d>) -> bool {
        static NO_PARAMS: std::sync::LazyLock<Params> = std::sync::LazyLock::new(Params::new);
        let env = Env {
            data: Some(data),
            row: Some(RowRef::Posting(row)),
            aggregates: &[],
            cells: &[],
            running: None,
            params: &NO_PARAMS,
            regexes: &self.regexes,
        };
        let bound = self.bound[at].get_or_init(|| {
            let label = &self.labels[at];
            crate::optimizer::bind_expr(&self.column.predicate, &label.params.clone().bind("label", label.label.as_str()))
        });
        matches!(bound.eval(&env), Ok(Value::Bool(true)))
    }
}

/// The value of the index column at `row`, read in place when the column supports it.
fn indexed_value<'r, 'd>(column: &ColumnRef, data: &'r Dataset<'d>, row: &'r Row<'d>) -> Option<std::borrow::Cow<'r, str>> {
    match column.borrow() {
        Borrow::Str(get) => get(data, row).map(std::borrow::Cow::Borrowed),
        _ => match column.value(data, RowRef::Posting(row)) {
            Value::Str(text) => Some(std::borrow::Cow::Owned(text)),
            _ => None,
        },
    }
}
