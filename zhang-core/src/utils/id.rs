use std::str::FromStr;

use sha256::digest;
use uuid::Uuid;
use zhang_ast::SpanInfo;

const DEFAULT_PATH: &str = "default_path";

pub trait FromSpan {
    fn from_span(span: &SpanInfo) -> Uuid;
    fn from_txn_posting(txn_id: &Uuid, idx: usize) -> Uuid;
    /// the `n`-th id derived from `id` for another directive at the same place, in a space of its own: it is no
    /// posting id ([`FromSpan::from_txn_posting`]) of a transaction with `id`
    fn derived(id: &Uuid, n: usize) -> Uuid;
}

impl FromSpan for Uuid {
    fn from_span(span: &SpanInfo) -> Uuid {
        let file = span.filename.as_ref().and_then(|buf| buf.to_str()).unwrap_or(DEFAULT_PATH);
        let string = digest(format!("{}-{}", file, span.start));
        Uuid::from_str(&string[0..32]).expect("invalid uuid")
    }

    fn from_txn_posting(txn_id: &Uuid, idx: usize) -> Uuid {
        let string = digest(format!("{}-{}", txn_id, idx));
        Uuid::from_str(&string[0..32]).expect("invalid uuid")
    }

    fn derived(id: &Uuid, n: usize) -> Uuid {
        let string = digest(format!("{}#{}", id, n));
        Uuid::from_str(&string[0..32]).expect("invalid uuid")
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use uuid::Uuid;
    use zhang_ast::SpanInfo;

    use crate::utils::id::FromSpan;

    #[test]
    fn should_generate_uuid_given_empty_file_name() {
        let empty_span = SpanInfo {
            start: 10,
            end: 0,
            content: "".to_string(),
            filename: None,
            ..SpanInfo::default()
        };
        assert_eq!(Uuid::from_span(&empty_span), Uuid::from_span(&empty_span))
    }

    #[test]
    fn should_generate_uuid_given_file_name() {
        let span = SpanInfo {
            start: 10,
            end: 0,
            content: "".to_string(),
            filename: Some(PathBuf::from("a.abc")),
            ..SpanInfo::default()
        };
        assert_eq!(Uuid::from_span(&span), Uuid::from_span(&span));

        assert_eq!(
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            }),
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            })
        );
    }

    #[test]
    fn should_generate_diff_uuid_given_diff_span() {
        assert_ne!(
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            }),
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: None,
                ..SpanInfo::default()
            })
        );
        assert_ne!(
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            }),
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.ab")),
                ..SpanInfo::default()
            })
        );

        assert_ne!(
            Uuid::from_span(&SpanInfo {
                start: 9,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            }),
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc")),
                ..SpanInfo::default()
            })
        );

        assert_ne!(
            Uuid::from_span(&SpanInfo {
                start: 9,
                end: 0,
                content: "".to_string(),
                filename: None,
                ..SpanInfo::default()
            }),
            Uuid::from_span(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: None,
                ..SpanInfo::default()
            })
        );
    }
}
