use sha256::{digest, try_digest};
use std::str::FromStr;
use uuid::Uuid;
use zhang_ast::SpanInfo;

const DEFAULT_PATH: &'static str = "default_path";
pub fn generate_uuid_from_span_info(span: &SpanInfo) -> Uuid {
    let file = span
        .filename
        .as_ref()
        .and_then(|buf| buf.to_str())
        .unwrap_or(DEFAULT_PATH);
    let string = digest(format!("{}-{}", &file, span.start));
    Uuid::from_str(&string[0..32]).unwrap()
}

#[cfg(test)]
mod test {
    use crate::utils::id::generate_uuid_from_span_info;
    use std::path::PathBuf;
    use zhang_ast::SpanInfo;

    #[test]
    fn should_generate_uuid_given_empty_file_name() {
        let empty_span = SpanInfo {
            start: 10,
            end: 0,
            content: "".to_string(),
            filename: None,
        };
        assert_eq!(
            generate_uuid_from_span_info(&empty_span),
            generate_uuid_from_span_info(&empty_span)
        )
    }

    #[test]
    fn should_generate_uuid_given_file_name() {
        let span = SpanInfo {
            start: 10,
            end: 0,
            content: "".to_string(),
            filename: Some(PathBuf::from("a.abc")),
        };
        assert_eq!(generate_uuid_from_span_info(&span), generate_uuid_from_span_info(&span));

        assert_eq!(
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            }),
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            })
        );
    }

    #[test]
    fn should_genereate_diff_uuid_given_diff_span() {
        assert_ne!(
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            }),
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: None
            })
        );
        assert_ne!(
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            }),
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.ab"))
            })
        );

        assert_ne!(
            generate_uuid_from_span_info(&SpanInfo {
                start: 9,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            }),
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: Some(PathBuf::from("a.abc"))
            })
        );

        assert_ne!(
            generate_uuid_from_span_info(&SpanInfo {
                start: 9,
                end: 0,
                content: "".to_string(),
                filename: None
            }),
            generate_uuid_from_span_info(&SpanInfo {
                start: 10,
                end: 0,
                content: "".to_string(),
                filename: None
            })
        );
    }
}
