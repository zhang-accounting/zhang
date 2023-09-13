pub mod constants;

pub mod database;
pub mod domains;
pub mod error;
pub mod ledger;
pub mod options;

pub mod exporter;
pub(crate) mod process;
pub mod transform;
pub mod utils;

pub mod store;
pub mod text;

pub type ZhangResult<T> = Result<T, ZhangError>;
pub use error::ZhangError;

#[cfg(test)]
mod test {
    use std::ops::Deref;
    use std::path::PathBuf;
    use std::sync::Arc;

    use glob::Pattern;
    use serde_json_path::JsonPath;
    use tempfile::tempdir;
    use zhang_ast::{Directive, Spanned};

    use crate::ledger::Ledger;
    use crate::text::parser::parse as parse_zhang;
    use crate::transform::{TransformResult, Transformer};
    use crate::ZhangResult;

    struct TestTransformer {}

    impl Transformer for TestTransformer {
        fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult> {
            let file = entry.join(endpoint);
            let string = std::fs::read_to_string(&file).unwrap();
            let result: Vec<Spanned<Directive>> = parse_zhang(&string, file).expect("cannot read file");
            Ok(TransformResult {
                directives: result,
                visited_files: vec![Pattern::new("example.zhang").unwrap()],
            })
        }
    }
    fn load_from_text(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        Ledger::load_with_database(temp_dir, "example.zhang".to_string(), Arc::new(TestTransformer {})).unwrap()
    }
    fn load_store(content: &str) -> StoreTest {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        StoreTest {
            ledger: Ledger::load_with_database(temp_dir, "example.zhang".to_string(), Arc::new(TestTransformer {})).unwrap(),
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
            let mut operations = ledger.operations();

            assert_eq!(operations.option("operating_currency").unwrap().unwrap().value, "USD");
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
            let mut operations = ledger.operations();

            let mut vec = operations.metas(MetaType::AccountMeta, "Assets:MyCard")?;
            assert_eq!(1, vec.len());
            let meta = vec.pop().unwrap();
            assert_eq!(meta.key, "a");
            assert_eq!(meta.value, "b");
            assert_eq!(meta.type_identifier, "Assets:MyCard");
            Ok(())
        }
    }
    mod account {
        use indoc::indoc;

        use crate::domains::schemas::AccountStatus;
        use crate::test::load_from_text;

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

            let mut operations = ledger.operations();

            let result = operations.single_account_balances("Assets:MyCard")?;
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

            let mut operations = ledger.operations();

            let lunch_balance = operations.single_account_balances("Expenses:Lunch")?.pop().unwrap();
            assert_eq!(lunch_balance.account, "Expenses:Lunch");
            assert_eq!(lunch_balance.balance_number, BigDecimal::from(50));
            assert_eq!(lunch_balance.balance_commodity, "CNY");

            let card_balance = operations.single_account_balances("Assets:MyCard")?.pop().unwrap();
            assert_eq!(card_balance.account, "Assets:MyCard");
            assert_eq!(card_balance.balance_number, BigDecimal::from(-50));
            assert_eq!(card_balance.balance_commodity, "CNY");
            Ok(())
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

            let mut operations = ledger.operations();
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

            let mut operations = ledger.operations();
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

            let mut operations = ledger.operations();
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

            let mut operations = ledger.operations();
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

            let mut operations = ledger.operations();
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

            let mut operations = ledger.operations();
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

        use crate::domains::schemas::ErrorType;
        use crate::test::load_from_text;

        mod close_non_zero_account {
            use indoc::indoc;

            use crate::domains::schemas::ErrorType;
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
                assert_eq!(error.error_type, ErrorType::CloseNonZeroAccount);
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
            assert_eq!(domain.error_type, ErrorType::AccountBalanceCheckError);
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

            let mut operations = ledger.operations();
            let timezone = operations.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone.value);
            Ok(())
        }

        #[test]
        fn should_fallback_to_use_system_timezone_given_invalid_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "MYZone"
                "#});

            let mut operations = ledger.operations();
            let timezone = operations.option("timezone")?.unwrap();
            assert_eq!(iana_time_zone::get_timezone().unwrap(), timezone.value);
            Ok(())
        }
        #[test]
        fn should_parse_user_timezone() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                    option "timezone" "Antarctica/South_Pole"
                "#});

            let mut operations = ledger.operations();
            let timezone = operations.option("timezone")?.unwrap();
            assert_eq!("Antarctica/South_Pole", timezone.value);
            assert_eq!(ledger.options.timezone, "Antarctica/South_Pole".parse().unwrap());
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
