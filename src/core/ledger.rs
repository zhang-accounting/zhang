use crate::core::models::Directive;
use crate::error::{AvaroError, AvaroResult};
use crate::parse_avaro;
use itertools::Itertools;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Ledger {
    directives: Vec<Directive>,
}

impl Ledger {
    pub fn load(entry: PathBuf) -> AvaroResult<Ledger> {
        let content = std::fs::read_to_string(entry)?;
        let directives =
            parse_avaro(&content).map_err(|it| AvaroError::PestError(it.to_string()))?;
        Ok(Self { directives })
    }

    pub fn apply(mut self, applier: impl Fn(Directive) -> Directive) -> Self {
        let vec = self.directives
            .into_iter()
            .map(|it| applier(it))
            .collect_vec();
        self.directives = vec;
        self
    }
}
