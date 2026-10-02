//! The directive-processing pipeline.
//!
//! A [`ProcessStage`] is a pure transform over the whole directive stream with an
//! error channel: `(directives, ctx) -> directives`, problems reported via
//! [`StageContext::emit_error`]. Core ops (as native Rust stages) and user plugins
//! (adapted from WASM) share this one interface; the executor runs them in
//! declaration order and re-sorts the stream after every stage, so a stage never
//! has to maintain ordering itself.

use std::collections::HashMap;

use log::debug;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, SpanInfo, Spanned};

use crate::domains::schemas::OptionDomain;
use crate::ledger::Ledger;
use crate::ZhangResult;

/// a problem reported by a stage; collected by the executor and materialized
/// into the store after the pipeline finishes
pub struct StageError {
    pub kind: ErrorKind,
    pub span: SpanInfo,
    pub metas: HashMap<String, String>,
}

/// context handed to every stage
pub struct StageContext<'a> {
    /// the ledger's resolved options
    pub options: &'a [OptionDomain],
    errors: Vec<StageError>,
}

impl<'a> StageContext<'a> {
    pub fn new(options: &'a [OptionDomain]) -> Self {
        Self { options, errors: vec![] }
    }

    /// report a problem without aborting the pipeline
    pub fn emit_error(&mut self, kind: ErrorKind, span: SpanInfo, metas: HashMap<String, String>) {
        self.errors.push(StageError { kind, span, metas });
    }

    /// consume the context and take the errors stages reported
    pub fn into_errors(self) -> Vec<StageError> {
        self.errors
    }
}

/// one step of the pipeline: transform the whole directive stream
pub trait ProcessStage {
    fn name(&self) -> &str;

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>>;
}

/// run stages in order; the stream is re-sorted after every stage
/// ("don't trust the stages" — beancount does the same after every plugin)
pub fn run_pipeline(stages: &[Box<dyn ProcessStage>], mut directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
    for stage in stages {
        debug!("running pipeline stage: {}", stage.name());
        directives = stage.process(directives, ctx)?;
        directives = Ledger::sort_directives_datetime(directives);
    }
    Ok(directives)
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::{Comment, Directive, SpanInfo, Spanned};

    use super::{run_pipeline, ProcessStage, StageContext};
    use crate::ZhangResult;

    fn span() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 0,
            content: "".to_string(),
            filename: None,
        }
    }

    struct AppendCommentStage(&'static str);
    impl ProcessStage for AppendCommentStage {
        fn name(&self) -> &str {
            "append-comment"
        }
        fn process(&self, mut directives: Vec<Spanned<Directive>>, _ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            directives.push(Spanned::new(Directive::Comment(Comment { content: self.0.to_owned() }), span()));
            Ok(directives)
        }
    }

    struct EmitErrorStage;
    impl ProcessStage for EmitErrorStage {
        fn name(&self) -> &str {
            "emit-error"
        }
        fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
            ctx.emit_error(ErrorKind::ParseInvalidMeta, span(), HashMap::default());
            Ok(directives)
        }
    }

    #[test]
    fn should_run_stages_in_order_and_collect_errors() {
        let stages: Vec<Box<dyn ProcessStage>> = vec![
            Box::new(AppendCommentStage("first")),
            Box::new(EmitErrorStage),
            Box::new(AppendCommentStage("second")),
        ];
        let mut ctx = StageContext::new(&[]);

        let out = run_pipeline(&stages, vec![], &mut ctx).unwrap();

        let comments: Vec<&str> = out
            .iter()
            .filter_map(|it| match &it.data {
                Directive::Comment(c) => Some(c.content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(comments, vec!["first", "second"]);

        let errors = ctx.into_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ErrorKind::ParseInvalidMeta);
    }
}
