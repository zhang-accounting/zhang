#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
pub use semver::Version;
use serde::{Deserialize, Serialize};
use zhang_ast::{Directive, Spanned};

use crate::plugin::http::{PluginRequest, PluginResponse};

pub mod capabilities;
pub mod host;
pub mod http;
pub mod router;
pub mod stage;
pub mod store;

/// indicate which type the plugin belongs to
/// the plugin can be multiple types
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
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

    /// the plugin exports `router`, which handles the HTTP requests to `/api/plugins/{name}` and the
    /// paths below it, e.g. to serve a custom report page or its data. The request is a
    /// [`http::PluginRequest`] and the response a [`http::PluginResponse`], both as JSON; see
    /// [`router`] for the host functions it can read the ledger with
    Router,

    /// a type this version of zhang does not know, e.g. one added by a newer zhang.
    /// A plugin declaring it still loads; registration ignores the type with a warning.
    #[serde(other)]
    Unknown,
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

    fn router(_: PluginRequest) -> PluginResponse {
        unimplemented!("plugin does not support router type")
    }
}
