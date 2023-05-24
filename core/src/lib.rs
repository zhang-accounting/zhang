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
        use crate::test::load_from_text;
        use indoc::indoc;

        #[tokio::test]
        async fn should_get_option() -> Result<(), Box<dyn std::error::Error>> {
            let ledger = load_from_text(indoc! {r#"
                 option "title" "Example"
            "#})
            .await;
            let mut operations = ledger.operations().await;

            let option = operations.options("title").await.unwrap().unwrap();
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

            let option = operations.options("title").await.unwrap().unwrap();
            assert_eq!(option.key, "title");
            assert_eq!(option.value, "Example2");
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
}
