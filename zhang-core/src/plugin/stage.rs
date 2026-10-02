//! Adapters exposing registered WASM plugins as pipeline stages.
//!
//! A plugin declaring `Processor` transforms the whole stream in one call; one
//! declaring `Mapper` is applied per directive. Both are ordinary
//! [`ProcessStage`]s, so user plugins and native core stages compose in one
//! pipeline, in declaration order.

use zhang_ast::{Directive, Spanned};

use crate::pipeline::{ProcessStage, StageContext};
use crate::plugin::store::RegisteredPlugin;
use crate::ZhangResult;

pub struct WasmProcessorStage {
    pub plugin: RegisteredPlugin,
}

impl ProcessStage for WasmProcessorStage {
    fn name(&self) -> &str {
        &self.plugin.name
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        self.plugin.execute_as_processor(directives, ctx.options)
    }
}

pub struct WasmMapperStage {
    pub plugin: RegisteredPlugin,
}

impl ProcessStage for WasmMapperStage {
    fn name(&self) -> &str {
        &self.plugin.name
    }

    fn process(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        self.plugin.execute_as_mapper(directives, ctx.options)
    }
}
