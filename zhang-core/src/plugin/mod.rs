use crate::plugin::http::PluginRequest;
pub use semver::Version;
use serde::{Deserialize, Serialize};
use zhang_ast::{Directive, Spanned};

pub mod http;
pub mod store;

/// indicate which type the plugin belongs to
/// the plugin can be multiple types
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum PluginType {
    /// the plugin can handle batches of directive, usually used to filter or combine directives, signature would be like [Plugin::processor]
    Processor,

    /// the plugin have the handler map directive to another directive, usually used to modify **single** directive
    /// the mapper signature would be like [Plugin::mapper]
    /// ```rust,ignore
    /// fn mapper(directive: Spanned<Directive>) -> Vec<Spanned<Directive>> {
    ///     // your logic here
    /// }
    /// ```
    Mapper,

    /// the plugin can handle the customized routes, usually used for new page's API
    /// like the request of URL `/api/plugins/{PLUGIN_NAME}/my-resources` will be forwarded to plugin's router by zhang-core
    Router,
}

pub trait Plugin {
    const NAME: &'static str;
    const VERSION: &'static str;

    /// indicate which types the plugin supports
    fn supported_type() -> Vec<PluginType>;

    fn processor(_: Vec<Spanned<Directive>>) -> Vec<Spanned<Directive>> {
        unimplemented!("plugin does not support processor type");
    }

    fn mapper(_: Spanned<Directive>) -> Vec<Spanned<Directive>> {
        unimplemented!("plugin does not support mapper type")
    }
}
