pub use error::ZhangError;

#[macro_use]
pub mod utils;

pub(crate) mod booking;
pub mod clock;
pub mod constants;
pub mod data_source;
pub mod data_type;
pub mod domains;
pub mod error;
pub mod inputs;
pub mod inventory;
pub mod ledger;
pub mod options;
pub mod pipeline;
#[cfg(feature = "plugin_runtime")]
pub mod plugin;
pub(crate) mod process;
pub mod store;

pub mod features;

pub use zhang_ast as ast;

pub type ZhangResult<T> = Result<T, ZhangError>;

#[cfg(test)]
mod test {
    use std::ops::Deref;
    use std::sync::Arc;

    use serde_json_path::JsonPath;
    use tempfile::tempdir;

    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::ledger::Ledger;

    fn load_from_text(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap()
    }
    fn load_store(content: &str) -> StoreTest {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        StoreTest {
            ledger: Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap(),
        }
    }

    struct StoreTest {
        ledger: Ledger,
    }

    impl StoreTest {
        pub fn assert_string(self, path: &str, expected_data: &str, msg: &str) -> Self {
            let operations = self.ledger.operations();
            let guard = operations.store.read().unwrap();
            let x = guard.deref();
            let value = serde_json::to_value(x).unwrap();
            let json_path = JsonPath::parse(path).unwrap();
            let node = json_path.query(&value).exactly_one().unwrap().as_str().unwrap();
            assert_eq!(node, expected_data, "{}", msg);
            self
        }
    }

    mod options {
        use indoc::indoc;
        use strum::IntoEnumIterator;

        use crate::options::BuiltinOption;
        use crate::test::{load_from_text, load_store};

        #[test]
        fn should_get_option() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
            "#})
            .assert_string("$.options.title", "Example", "");
            Ok(())
        }

        #[test]
        fn should_get_latest_option_given_same_options() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
            "#})
            .assert_string("$.options.title", "Example2", "");
            Ok(())
        }

        #[test]
        fn should_get_default_options() -> Result<(), Box<dyn std::error::Error>> {
            load_store(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
            "#})
            .assert_string("$.options.title", "Example2", "")
            .assert_string("$.options.operating_currency", "CNY", "")
            .assert_string("$.options.default_rounding", "RoundDown", "")
            .assert_string("$.options.default_balance_tolerance_precision", "2", "");
            Ok(())
        }

        #[test]
        fn should_be_override_by_user_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "operating_currency" "USD"
            "#});
            let operations = ledger.operations();

            assert_eq!(operations.option::<String>("operating_currency").unwrap().unwrap(), "USD");
            Ok(())
        }

        #[test]
        fn should_get_all_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
                 option "url" "url here"
            "#});
            let mut operations = ledger.operations();

            let options = operations.options().unwrap();
            assert_eq!(BuiltinOption::iter().count() + 2, options.len());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("title")).count());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("url")).count());
            Ok(())
        }
    }

    mod meta {
        use indoc::indoc;

        use crate::domains::schemas::MetaType;
        use crate::test::load_from_text;

        #[test]
        fn should_get_account_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                  a: "b"
            "#});
            let operations = ledger.operations();

            let mut vec = operations.metas(MetaType::AccountMeta, "Assets:MyCard")?;
            assert_eq!(1, vec.len());
            let meta = vec.pop().unwrap();
            assert_eq!(meta.key, "a");
            assert_eq!(meta.value, "b");
            assert_eq!(meta.type_identifier, "Assets:MyCard");
            Ok(())
        }

        #[test]
        fn should_keep_posting_meta_per_posting() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                1970-01-01 open Assets:Cash
                1970-01-01 open Expenses:Food
                  budget: food
                2024-01-01 budget food CNY

                2024-01-02 * "Cafe" "lunch"
                  memo: "m"
                  document: "receipts/transaction.pdf"
                  Assets:Cash -5 CNY
                    receipt: "r1"
                  Expenses:Food 5 CNY
                    document: "receipts/posting.pdf"
                    b: "2"
                    a: "1"
                    a: "0"
            "#});
            let operations = ledger.operations();
            let store = operations.read();
            assert!(store.errors.is_empty(), "{:?}", store.errors);
            let txn = store.transactions.values().next().unwrap().clone();
            let pairs = |metas: &[crate::store::PostingMetaDomain]| metas.iter().map(|it| format!("{}={}", it.key, it.value)).collect::<Vec<_>>();
            let postings = txn.postings.iter().map(|posting| pairs(&posting.metas)).collect::<Vec<_>>();
            // sorted by key, the values of a repeated key in ledger order
            assert_eq!(postings, vec![vec!["receipt=r1"], vec!["a=1", "a=0", "b=2", "document=receipts/posting.pdf"]]);
            let stored = store.postings.iter().map(|posting| pairs(&posting.metas)).collect::<Vec<_>>();
            assert_eq!(stored, postings);

            // the transaction's own metadata is unchanged
            let mut documents = store
                .documents
                .iter()
                .filter(|it| it.document_type.as_trx() == Some(txn.id.to_string()))
                .map(|it| it.path.clone())
                .collect::<Vec<_>>();
            drop(store);
            let mut metas = operations
                .metas(MetaType::TransactionMeta, txn.id.to_string())?
                .into_iter()
                .map(|meta| format!("{}={}", meta.key, meta.value))
                .collect::<Vec<_>>();
            metas.sort();
            assert_eq!(metas, vec!["document=receipts/transaction.pdf", "memo=m"]);

            // a posting's document is a document of its transaction too
            documents.sort();
            assert_eq!(documents, vec!["receipts/posting.pdf", "receipts/transaction.pdf"]);

            // budgets read the account metadata as before
            let detail = operations.budget_month_detail("food", 202401)?.expect("budget month");
            assert_eq!(detail.activity_amount.number, bigdecimal::BigDecimal::from(5));
            Ok(())
        }

        /// Directives go to a WASM plugin, and come back from it, as JSON.
        #[test]
        fn postings_without_meta_from_an_older_plugin_still_deserialise() {
            use zhang_ast::{Directive, Spanned};

            use crate::data_type::text::ZhangDataType;
            use crate::data_type::DataType;

            let directives = ZhangDataType {}
                .transform(
                    "2024-01-02 * \"Cafe\"\n  memo: \"m\"\n  Assets:Cash -5 CNY\n    receipt: \"r1\"\n  Expenses:Food 5 CNY\n".to_owned(),
                    None,
                )
                .unwrap();
            let mut json = serde_json::to_value(&directives).unwrap();
            let postings = json[0]["data"]["Transaction"]["postings"].as_array_mut().unwrap();
            assert_eq!(
                postings[0]["meta"]["inner"]["receipt"][0]["QuoteString"], "r1",
                "a plugin receives the posting metadata"
            );

            // what a plugin built against a zhang-ast without posting metadata returns
            for posting in postings {
                posting.as_object_mut().unwrap().remove("meta");
            }
            let returned: Vec<Spanned<Directive>> = serde_json::from_value(json).expect("an older plugin's directives deserialise");
            let (Directive::Transaction(returned), Directive::Transaction(original)) = (&returned[0].data, &directives[0].data) else {
                unreachable!()
            };
            assert!(returned.postings.iter().all(|posting| posting.meta.clone().get_flatten().is_empty()));
            assert_eq!(returned.meta, original.meta);
            assert_eq!(returned.postings.len(), original.postings.len());
        }
    }
    mod account {
        use indoc::indoc;

        use crate::domains::schemas::AccountStatus;
        use crate::test::{load_from_text, load_store};

        #[test]
        fn should_closed_account() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                1970-01-02 close Assets:MyCard
            "#});

            let mut operations = ledger.operations();
            let account = operations.account("Assets:MyCard")?.unwrap();
            assert_eq!(account.status, AccountStatus::Close);
            assert_eq!(account.alias, None);
            Ok(())
        }

        #[test]
        fn should_get_alias_from_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                  alias: "MyCardAliasName"
            "#});

            let mut operations = ledger.operations();
            let account = operations.account("Assets:MyCard")?.unwrap();
            assert_eq!(account.alias.unwrap(), "MyCardAliasName");
            Ok(())
        }

        #[test]
        fn should_return_all_accounts() {
            let ledger = load_store(indoc! {r#"
                1970-01-01 commodity USD
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:A

                1970-01-02 "Apple Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A

                1970-01-02 "Origan Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A
            "#})
            .ledger;
            let mut operations = ledger.operations();
            let result = operations.all_accounts().unwrap();
            assert!(result.contains(&"Assets:A".to_owned()));
            assert!(result.contains(&"Expenses:A".to_owned()));
        }
    }

    mod account_balance {
        use bigdecimal::BigDecimal;
        use indoc::indoc;

        use crate::test::load_from_text;

        #[test]
        fn should_return_zero_balance_given_zero_directive() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
            "#});

            let operations = ledger.operations();

            let result = operations.single_account_latest_balances("Assets:MyCard")?;
            assert_eq!(0, result.len());

            Ok(())
        }
        #[test]
        fn should_return_correct_balance_given_txn() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                1970-01-01 open Expenses:Lunch
                1970-01-02 "KFC" "Crazy Thursday"
                  Assets:MyCard -50 CNY
                  Expenses:Lunch 50 CNY
            "#});

            let operations = ledger.operations();

            let lunch_balance = operations.single_account_latest_balances("Expenses:Lunch")?.pop().unwrap();
            assert_eq!(lunch_balance.account, "Expenses:Lunch");
            assert_eq!(lunch_balance.balance.number, BigDecimal::from(50));
            assert_eq!(lunch_balance.balance.commodity, "CNY");

            let card_balance = operations.single_account_latest_balances("Assets:MyCard")?.pop().unwrap();
            assert_eq!(card_balance.account, "Assets:MyCard");
            assert_eq!(card_balance.balance.number, BigDecimal::from(-50));
            assert_eq!(card_balance.balance.commodity, "CNY");
            Ok(())
        }

        #[test]
        fn should_get_correct_balance_after_pad_and_trx() {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:B
                1970-01-01 open Equity:Open-Balancing

                2023-01-01 balance Assets:A 3000 CNY with pad Equity:Open-Balancing

                2023-01-02 "Shopping" ""
                    Assets:A -30 CNY
                    Expenses:B
            "#});

            let operations = ledger.operations();

            let mut result = operations.single_account_latest_balances("Assets:A").unwrap();
            let balance = result.pop().unwrap();
            assert_eq!(balance.balance.number, BigDecimal::from(2970i32));
            assert_eq!(balance.balance.commodity, "CNY");
        }
    }
    mod commodity {
        use indoc::indoc;

        use crate::test::load_from_text;

        #[test]
        fn should_get_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("CNY")?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(2, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_not_get_non_exist_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("USD")?;
            assert!(commodity.is_none());
            Ok(())
        }

        #[test]
        fn should_get_correct_precision_given_override_default_precision() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("CNY")?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_get_info_from_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                  precision: "3"
                  prefix: "¥"
                  suffix: "CNY"
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("CNY")?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!("¥", commodity.prefix.unwrap());
            assert_eq!("CNY", commodity.suffix.unwrap());
            Ok(())
        }
        #[test]
        fn should_meta_precision_have_higher_priority() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
                  precision: "4"
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("CNY")?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[test]
        fn should_work_with_same_default_operating_currency_and_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "operating_currency" "CNY"
                1970-01-01 commodity CNY
                  precision: "4"
            "#});

            let operations = ledger.operations();
            let commodity = operations.commodity("CNY")?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }
    }
    mod error {
        use indoc::indoc;
        use zhang_ast::error::ErrorKind;

        use crate::test::load_from_text;

        mod close_non_zero_account {
            use indoc::indoc;
            use zhang_ast::error::ErrorKind;

            use crate::test::load_from_text;

            #[test]
            fn should_not_raise_error() -> Result<(), Box<dyn std::error::Error>> {
                let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard
                    1970-01-03 close Assets:MyCard
                "#});

                let mut operations = ledger.operations();
                let errors = operations.errors()?;
                assert_eq!(errors.len(), 0);
                Ok(())
            }
            #[test]
            fn should_raise_error() -> Result<(), Box<dyn std::error::Error>> {
                let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard
                    1970-01-01 open Expenses:Lunch
                    1970-01-02 "KFC" "Crazy Thursday"
                      Assets:MyCard -50 CNY
                      Expenses:Lunch 50 CNY

                    1970-01-03 close Assets:MyCard
                "#});

                let mut operations = ledger.operations();
                let mut errors = operations.errors()?;
                assert_eq!(errors.len(), 1);
                let error = errors.pop().unwrap();
                assert_eq!(error.error_type, ErrorKind::CloseNonZeroAccount);
                Ok(())
            }
        }

        #[test]
        fn should_raise_non_balance_error_only() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard CNY
                    1970-01-03 balance Assets:MyCard 10 CNY
                "#});

            let mut operations = ledger.operations();
            let mut errors = operations.errors()?;
            assert_eq!(errors.len(), 1);
            let domain = errors.pop().unwrap();
            assert_eq!(domain.error_type, ErrorKind::AccountBalanceCheckError);
            assert_eq!(domain.metas.get("account_name").unwrap(), "Assets:MyCard");
            Ok(())
        }
    }
    mod timezone {
        use indoc::indoc;

        use crate::test::load_from_text;

        #[test]
        fn should_get_system_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    1970-01-01 open Assets:MyCard CNY
                "#});

            let operations = ledger.operations();
            let timezone: String = operations.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone);
            Ok(())
        }

        #[test]
        fn should_fallback_to_use_system_timezone_given_invalid_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "MYZone"
                "#});

            let operations = ledger.operations();
            let timezone: String = operations.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone);
            Ok(())
        }
        #[test]
        fn should_parse_user_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "Antarctica/South_Pole"
                "#});

            let operations = ledger.operations();
            let timezone: String = operations.option("timezone")?.unwrap();
            assert_eq!("Antarctica/South_Pole", timezone);
            assert_eq!(ledger.options.timezone, "Antarctica/South_Pole".parse().unwrap());
            Ok(())
        }

        /// the instants (in UTC) of the stored transactions, keyed by narration
        fn transaction_instants(ledger: &crate::ledger::Ledger) -> std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> {
            let operations = ledger.operations();
            let store = operations.read();
            store
                .transactions
                .values()
                .map(|trx| (trx.narration.clone().unwrap_or_default(), trx.datetime.to_utc()))
                .collect()
        }

        fn utc(text: &str) -> chrono::DateTime<chrono::Utc> {
            text.parse().unwrap()
        }

        #[test]
        fn should_load_a_local_time_skipped_by_dst() -> Result<(), Box<dyn std::error::Error>> {
            // 02:30 does not exist in New York on 2023-03-12: clocks jump 02:00 EST -> 03:00 EDT
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/New_York"
                option "operating_currency" "USD"

                2023-01-01 open Assets:Cash USD
                2023-01-01 open Expenses:Food USD

                2023-03-12 02:30:00 "gap" "this local time does not exist"
                  Assets:Cash -1 USD
                  Expenses:Food
            "#});

            assert!(ledger.operations().errors()?.is_empty());
            let instants = transaction_instants(&ledger);
            // read with the offset before the transition: 02:30 EST = 07:30 UTC = 03:30 EDT
            assert_eq!(instants["this local time does not exist"], utc("2023-03-12T07:30:00Z"));

            let operations = ledger.operations();
            let store = operations.read();
            assert_eq!(store.postings.len(), 2);
            assert!(store
                .postings
                .iter()
                .all(|posting| posting.trx_datetime.to_utc() == utc("2023-03-12T07:30:00Z")));
            Ok(())
        }

        #[test]
        fn should_load_ambiguous_and_midnight_skipped_local_times() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/New_York"
                option "operating_currency" "USD"

                2023-01-01 open Assets:Cash USD
                2023-01-01 open Expenses:Food USD

                2023-11-05 01:30:00 "overlap" "this local time happens twice"
                  Assets:Cash -1 USD
                  Expenses:Food
            "#});
            assert!(ledger.operations().errors()?.is_empty());
            // the earlier of 01:30 EDT and 01:30 EST
            assert_eq!(transaction_instants(&ledger)["this local time happens twice"], utc("2023-11-05T05:30:00Z"));

            // Santiago switches at midnight, so 2023-09-03 has no 00:00 for a date-only directive
            let ledger = load_from_text(indoc! {r#"
                option "timezone" "America/Santiago"
                option "operating_currency" "CLP"

                2023-09-03 open Assets:Cash CLP
                2023-09-03 open Expenses:Food CLP

                2023-09-03 "midnight" "this day has no midnight"
                  Assets:Cash -1 CLP
                  Expenses:Food
            "#});
            assert!(ledger.operations().errors()?.is_empty());
            // 00:00 -04 = 04:00 UTC = 01:00 -03
            assert_eq!(transaction_instants(&ledger)["this day has no midnight"], utc("2023-09-03T04:00:00Z"));
            Ok(())
        }
    }

    mod transaction {
        use indoc::indoc;

        use crate::test::load_store;

        #[test]
        fn should_get_all_payees() {
            let ledger = load_store(indoc! {r#"
                1970-01-01 commodity USD
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:A

                1970-01-02 "Apple Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A

                1970-01-02 "Origan Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A
            "#})
            .ledger;
            let mut operations = ledger.operations();
            let result = operations.all_payees().unwrap();
            assert!(result.contains(&"Origan Inc".to_owned()));
            assert!(result.contains(&"Apple Inc".to_owned()));
        }

        #[test]
        fn should_remove_duplicated_payees() {
            let ledger = load_store(indoc! {r#"
                1970-01-01 commodity USD
                1970-01-01 open Assets:A
                1970-01-01 open Expenses:A

                1970-01-02 "Apple Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A

                1970-01-02 "Apple Inc" "iPhone 15"
                  Assets:A -1000 USD
                  Expenses:A
            "#})
            .ledger;
            let mut operations = ledger.operations();
            let result = operations.all_payees().unwrap();
            assert!(result.contains(&"Apple Inc".to_owned()));
            assert_eq!(1, result.len());
        }
    }
}
