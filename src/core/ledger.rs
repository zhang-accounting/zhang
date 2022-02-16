use crate::core::models::Directive;
use crate::error::{ZhangError, ZhangResult};
use crate::parse_zhang;
use itertools::Itertools;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Ledger {
    pub(crate) directives: Vec<Directive>,
}

impl Ledger {
    pub fn load(entry: PathBuf) -> ZhangResult<Ledger> {
        let content = std::fs::read_to_string(entry)?;
        let directives =
            parse_zhang(&content).map_err(|it| ZhangError::PestError(it.to_string()))?;
        Ok(Self { directives })
    }

    pub fn apply(mut self, applier: impl Fn(Directive) -> Directive) -> Self {
        let vec = self.directives.into_iter().map(applier).collect_vec();
        self.directives = vec;
        self
    }
}
