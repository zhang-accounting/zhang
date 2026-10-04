//! Budget directives in beancount files (#500). Beancount has no budget directives; zhang writes them as
//! `custom` directives, and the only form beancount itself accepts is a quoted type, quoted names, a quoted
//! commodity for `budget` and a plain amount for `budget-add` and `budget-transfer`. The beancount data type
//! reads that form as zhang's budget directives, writes it, and export → parse is an identity. The unquoted
//! form earlier versions of zhang wrote, `custom budget Food CNY`, is still read and is written quoted.
//!
//! Every text below was checked with `bean-check` (beancount 3.2.3): [`QUOTED`] passes, and each budget line
//! of [`UNQUOTED`] is an `Invalid token`.

use beancount::Beancount;
use zhang_ast::*;
use zhang_core::data_type::DataType;

/// The four budget directives, with metadata, as beancount accepts them.
const QUOTED: &str = "2024-01-01 custom \"budget\" \"Food\" \"CNY\"
  alias: \"外食\"
  category: \"生活开销｜55%\"
2024-01-01 custom \"budget-add\" \"Food\" 2000 CNY
2024-01-20 custom \"budget-transfer\" \"Fun\" \"Food\" 100.5 CNY
2024-12-31 custom \"budget-close\" \"Food\"
  note: \"year end\"";

/// The same directives as earlier versions of zhang wrote them, which beancount rejects.
const UNQUOTED: &str = "2024-01-01 custom budget Food CNY
  alias: \"外食\"
  category: \"生活开销｜55%\"
2024-01-01 custom budget-add Food 2000 CNY
2024-01-20 custom budget-transfer Fun Food 100.5 CNY
2024-12-31 custom budget-close Food
  note: \"year end\"";

fn read(text: &str) -> Vec<Directive> {
    Beancount::default()
        .transform(text.to_owned(), None)
        .unwrap()
        .into_iter()
        .map(|it| it.data)
        .collect()
}

fn write(directives: Vec<Directive>) -> String {
    directives
        .into_iter()
        .map(|directive| Beancount::default().export(Spanned::new(directive, SpanInfo::default())))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn budgets_are_read_as_budget_directives_and_written_in_the_form_beancount_accepts() {
    let directives = read(QUOTED);
    assert_eq!(directives.len(), 4);
    assert!(directives.iter().all(|it| !matches!(it, Directive::Custom(..))), "{:?}", directives);
    assert_eq!(write(directives), QUOTED);
}

#[test]
fn the_quoted_form_round_trips() {
    let directives = read(QUOTED);
    let written = write(directives.clone());
    assert_eq!(read(&written), directives);
    assert_eq!(write(read(&written)), written);
}

#[test]
fn the_unquoted_form_is_read_the_same_and_written_quoted() {
    assert_eq!(read(UNQUOTED), read(QUOTED));
    assert_eq!(write(read(UNQUOTED)), QUOTED);
}
