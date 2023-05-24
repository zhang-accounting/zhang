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
}
