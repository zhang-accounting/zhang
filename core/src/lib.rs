pub mod constants;
pub mod database;
pub mod domains;
pub mod error;
pub mod exporter;
pub mod ledger;
pub mod options;
pub(crate) mod process;
pub mod transform;
pub mod utils;

pub type ZhangResult<T> = Result<T, ZhangError>;
pub use error::ZhangError;

#[cfg(test)]
mod test {
    use crate::ledger::Ledger;
    use crate::transform::{TransformResult, Transformer};
    use crate::ZhangResult;
    use glob::Pattern;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;
    use text_transformer::parse as parse_zhang;
    use zhang_ast::{Directive, Spanned};

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
    async fn load_from_text(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(&example, content).unwrap();
        Ledger::load_with_database(temp_dir, "example.zhang".to_string(), None, Arc::new(TestTransformer {}))
            .await
            .unwrap()
    }

    mod options {
        use crate::options::DEFAULT_OPTIONS;
        use crate::test::load_from_text;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_option() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            let option = operations.option("title").await.unwrap().unwrap();
            assert_eq!(option.key, "title");
            assert_eq!(option.value, "Example");
            Ok(())
        }

        #[tokio::test]
        async fn should_get_latest_option_given_same_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            let option = operations.option("title").await.unwrap().unwrap();
            assert_eq!(option.key, "title");
            assert_eq!(option.value, "Example2");
            Ok(())
        }

        #[tokio::test]
        async fn should_get_default_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            assert_eq!(operations.option("operating_currency").await.unwrap().unwrap().value, "CNY");
            assert_eq!(operations.option("default_rounding").await.unwrap().unwrap().value, "RoundDown");
            assert_eq!(operations.option("default_balance_tolerance_precision").await.unwrap().unwrap().value, "2");
            Ok(())
        }
        #[tokio::test]
        async fn should_be_override_by_user_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "operating_currency" "USD"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            assert_eq!(operations.option("operating_currency").await.unwrap().unwrap().value, "USD");
            Ok(())
        }

        #[tokio::test]
        async fn should_get_all_options() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
                 option "title" "Example2"
                 option "url" "url here"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            let options = operations.options().await.unwrap();
            assert_eq!(DEFAULT_OPTIONS.len() + 2, options.len());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("title")).count());
            assert_eq!(1, options.iter().filter(|it| it.key.eq("url")).count());
            Ok(())
        }
    }

    mod meta {
        use crate::test::load_from_text;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_account_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                  a: "b"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            let mut vec = operations.metas("AccountMeta", "Assets:MyCard").await?;
            assert_eq!(1, vec.len());
            let meta = vec.pop().unwrap();
            assert_eq!(meta.key, "a");
            assert_eq!(meta.value, "b");
            assert_eq!(meta.type_identifier, "Assets:MyCard");
            Ok(())
        }
    }

    mod account_balance {
        use crate::test::load_from_text;
        use bigdecimal::BigDecimal;
        use indoc::indoc;

        #[tokio::test]
        async fn should_return_zero_balance_given_zero_directive() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
            "#})
            .await;

            let mut operations = ledger.operations().await;

            let result = operations.account_balances().await?;
            assert_eq!(0, result.len());

            Ok(())
        }
        #[tokio::test]
        async fn should_return_correct_balance_given_txn() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 open Assets:MyCard
                1970-01-01 open Expenses:Lunch
                1970-01-02 "KFC" "Crazy Thursday"
                  Assets:MyCard -50 CNY
                  Expenses:Lunch 50 CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;

            let mut result = operations.account_balances().await?;
            assert_eq!(2, result.len());

            let lunch_balance = result.pop().unwrap();
            assert_eq!(lunch_balance.account, "Expenses:Lunch");
            assert_eq!(lunch_balance.balance_number.0, BigDecimal::from(50));
            assert_eq!(lunch_balance.balance_commodity, "CNY");

            let card_balance = result.pop().unwrap();
            assert_eq!(card_balance.account, "Assets:MyCard");
            assert_eq!(card_balance.balance_number.0, BigDecimal::from(-50));
            assert_eq!(card_balance.balance_commodity, "CNY");
            Ok(())
        }
    }
    mod commodity {
        use crate::test::load_from_text;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(2, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[tokio::test]
        async fn should_not_get_non_exist_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("USD").await?;
            assert!(commodity.is_none());
            Ok(())
        }

        #[tokio::test]
        async fn should_get_correct_precision_given_override_default_precision() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[tokio::test]
        async fn should_get_info_from_meta() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                1970-01-01 commodity CNY
                  precision: "3"
                  prefix: "¥"
                  suffix: "CNY"
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(3, commodity.precision);
            assert_eq!("¥", commodity.prefix.unwrap());
            assert_eq!("CNY", commodity.suffix.unwrap());
            Ok(())
        }
        #[tokio::test]
        async fn should_meta_precision_have_higher_priority() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "default_commodity_precision" "3"
                1970-01-01 commodity CNY
                  precision: "4"
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }

        #[tokio::test]
        async fn should_work_with_same_default_operating_currency_and_commodity() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                option "operating_currency" "CNY"
                1970-01-01 commodity CNY
                  precision: "4"
            "#})
            .await;

            let mut operations = ledger.operations().await;
            let commodity = operations.commodity("CNY").await?.unwrap();
            assert_eq!("CNY", commodity.name);
            assert_eq!(4, commodity.precision);
            assert_eq!(None, commodity.prefix);
            assert_eq!(None, commodity.suffix);
            Ok(())
        }
    }
}
