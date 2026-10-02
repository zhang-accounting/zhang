#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
pub use semver::Version;
use serde::{Deserialize, Serialize};

pub mod capabilities;
pub mod stage;
pub mod store;

/// indicate which type the plugin belongs to
/// the plugin can be multiple types
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum PluginType {
    /// the plugin exports `processor`, which takes the whole directive stream and returns a new one,
    /// usually used to filter or combine directives
    Processor,

    /// the plugin exports `mapper`, which maps a **single** directive to any number of directives,
    /// usually used to modify directives one by one
    Mapper,

    /// a type this version of zhang does not know, e.g. one added by a newer zhang.
    /// A plugin declaring it still loads; registration ignores the type with a warning.
    ///
    /// The retired `Router` type lands here too: it was registered but never ran.
    #[serde(other)]
    Unknown,
}
