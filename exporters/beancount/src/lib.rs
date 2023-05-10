use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use beancount_transformer::directives::{BalanceDirective, PadDirective};
use itertools::Itertools;
use text_exporter::{append_meta, TextExportable, TextExporter};
use zhang_ast::{Balance, Date, Directive, Include, Meta, ZhangString};
use zhang_core::exporter::{AppendableExporter, Exporter};
use zhang_core::ledger::Ledger;
use zhang_core::utils::has_path_visited;
use zhang_core::ZhangResult;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

pub struct BeancountExporter {}

impl AppendableExporter for BeancountExporter {
    fn append_directives(&self, ledger: &Ledger, file: PathBuf, directives: Vec<Directive>) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;
        let endpoint = entry.join(file);

        create_folder_if_not_exist(&endpoint);

        if !has_path_visited(&ledger.visited_files, &endpoint) {
            let path = match endpoint.strip_prefix(entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => endpoint.to_str().unwrap(),
            };
            self.append_directives(
                ledger,
                entry.join(main_file_endpoint),
                vec![Directive::Include(Include {
                    file: ZhangString::QuoteString(path.to_string()),
                })],
            )?;
        }
        let directive_content = format!("\n{}\n", directives.into_iter().map(|it| self.export_directive(it)).join("\n"));
        let mut ledger_base_file = OpenOptions::new().append(true).create(true).open(&endpoint).unwrap();
        Ok(ledger_base_file.write_all(directive_content.as_bytes())?)
    }
}

impl Exporter for BeancountExporter {
    type Output = String;

    fn export_directive(&self, directive: Directive) -> Self::Output {
        let text_exporter = TextExporter {};
        let directive = convert_datetime_to_date(directive);
        match directive {
            Directive::Balance(balance) => match balance {
                Balance::BalanceCheck(check) => {
                    let balance_directive = BalanceDirective {
                        date: check.date,
                        account: check.account,
                        amount: check.amount,

                        meta: check.meta,
                    };
                    BeancountOnlyExportable::export(balance_directive)
                }
                Balance::BalancePad(pad) => {
                    let balance_date = pad.date.naive_date();
                    let pad_date = balance_date.pred_opt().unwrap_or(balance_date);
                    let pad_directive = PadDirective {
                        date: Date::Date(pad_date),
                        account: pad.account.clone(),
                        pad: pad.pad,
                        meta: Meta::default(),
                    };
                    let balance_directive = BalanceDirective {
                        date: pad.date,
                        account: pad.account,
                        amount: pad.amount,

                        meta: pad.meta,
                    };
                    vec![
                        BeancountOnlyExportable::export(pad_directive),
                        BeancountOnlyExportable::export(balance_directive),
                    ]
                    .join("\n")
                }
            },
            _ => text_exporter.export_directive(directive),
        }
    }
}

trait BeancountOnlyExportable {
    fn export(self) -> String;
}

impl BeancountOnlyExportable for BalanceDirective {
    fn export(self) -> String {
        let line = vec![
            TextExportable::export(self.date),
            "balance".to_string(),
            TextExportable::export(self.account),
            TextExportable::export(self.amount),
        ]
        .join(" ");
        append_meta(self.meta, line)
    }
}

impl BeancountOnlyExportable for PadDirective {
    fn export(self) -> String {
        let line = vec![
            TextExportable::export(self.date),
            "pad".to_string(),
            TextExportable::export(self.account),
            TextExportable::export(self.pad),
        ]
        .join(" ");
        append_meta(self.meta, line)
    }
}

macro_rules! convert_to_datetime {
    ($directive: expr) => {
        if let Date::Datetime(datetime) = $directive.date {
            let (date, time) = (datetime.date(), datetime.time());
            $directive.date = Date::Date(date);
            $directive
                .meta
                .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
            $directive
        } else {
            $directive
        }
    };
}

fn convert_datetime_to_date(directive: Directive) -> Directive {
    match directive {
        Directive::Open(mut directive) => Directive::Open(convert_to_datetime!(directive)),
        Directive::Close(mut directive) => Directive::Close(convert_to_datetime!(directive)),
        Directive::Commodity(mut directive) => Directive::Commodity(convert_to_datetime!(directive)),
        Directive::Transaction(mut directive) => Directive::Transaction(convert_to_datetime!(directive)),
        Directive::Balance(mut directive) => Directive::Balance(match &mut directive {
            Balance::BalanceCheck(check) => match check.date {
                Date::Date(_) => directive,
                Date::DateHour(date_hour) => {
                    let (date, time) = (date_hour.date(), date_hour.time());
                    check.date = Date::Date(date);
                    check
                        .meta
                        .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
                    directive
                }
                Date::Datetime(datetime) => {
                    let (date, time) = (datetime.date(), datetime.time());
                    check.date = Date::Date(date);
                    check
                        .meta
                        .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
                    directive
                }
            },
            Balance::BalancePad(pad) => match pad.date {
                Date::Date(_) => directive,
                Date::DateHour(date_hour) => {
                    let (date, time) = (date_hour.date(), date_hour.time());
                    pad.date = Date::Date(date);
                    pad.meta
                        .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
                    directive
                }
                Date::Datetime(datetime) => {
                    let (date, time) = (datetime.date(), datetime.time());
                    pad.date = Date::Date(date);
                    pad.meta
                        .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
                    directive
                }
            },
        }),
        Directive::Note(mut directive) => Directive::Note(convert_to_datetime!(directive)),
        Directive::Document(mut directive) => Directive::Document(convert_to_datetime!(directive)),
        Directive::Price(mut directive) => Directive::Price(convert_to_datetime!(directive)),
        Directive::Event(mut directive) => Directive::Event(convert_to_datetime!(directive)),
        Directive::Custom(mut directive) => Directive::Custom(convert_to_datetime!(directive)),
        _ => directive,
    }
}

#[cfg(test)]
mod test {
    use crate::BeancountExporter;
    use beancount_transformer::{parse, BeancountOnlyDirective};
    use indoc::indoc;
    use std::str::FromStr;
    use zhang_ast::{Account, Balance, BalancePad, Date, Directive};
    use zhang_core::exporter::Exporter;

    macro_rules! test_parse_zhang {
        ($content: expr) => {{
            let directive = parse($content, None).unwrap().into_iter().next().unwrap().data;
            directive.left().unwrap()
        }};
    }
    macro_rules! test_parse_bc {
        ($content: expr) => {{
            let directive = parse($content, None).unwrap().into_iter().next().unwrap().data;
            dbg!(&directive);
            directive.right().unwrap()
        }};
    }

    #[test]
    fn should_keep_time_into_meta_for_open_directive() {
        let mut directive = test_parse_zhang! {"1970-01-01 open Assets:BankAccount"};
        match &mut directive {
            Directive::Open(ref mut open) => open.date = Date::Datetime(open.date.naive_date().and_hms_nano_opt(1, 1, 1, 0).unwrap()),
            _ => unreachable!(),
        }

        let beancount_exporter = BeancountExporter {};
        assert_eq!(
            indoc! {r#"
                1970-01-01 open Assets:BankAccount
                  time: "01:01:01"
            "#}
            .trim(),
            beancount_exporter.export_directive(directive),
            "should persist time into meta"
        );
    }

    #[test]
    fn should_convert_to_pad_and_balance_directive_given_balance_pad_directive() {
        let directive = test_parse_bc! {"1970-01-02 balance Assets:BankAccount 2 CNY"};
        let directive = match directive {
            BeancountOnlyDirective::Balance(check) => Directive::Balance(Balance::BalancePad(BalancePad {
                date: check.date,
                account: check.account,
                amount: check.amount,
                pad: Account::from_str("Equity:Open-Balances").unwrap(),
                meta: Default::default(),
            })),
            _ => unreachable!(),
        };

        let beancount_exporter = BeancountExporter {};
        assert_eq!(
            indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
                1970-01-02 balance Assets:BankAccount 2 CNY
            "#}
            .trim(),
            beancount_exporter.export_directive(directive),
        );
    }
}
