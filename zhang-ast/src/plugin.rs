//! Types of the WASM plugin ABI shared by the host (zhang-core) and the plugin SDK
//! (`zhang-plugin-sdk`), so both read and write one serde representation.

#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::{Deserialize, Serialize};

/// What a plugin runs as. A plugin lists the types it supports in its `supported_type` export, as a
/// JSON array such as `["Processor", "Router"]`; a plugin can be several types at once.
///
/// This representation is part of plugin ABI v1: a variant is never renamed or removed.
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum PluginType {
    /// the plugin exports `processor`, which receives the whole directive stream as JSON and
    /// returns the new stream, e.g. to filter, combine or add directives
    Processor,

    /// the plugin exports `mapper`, which receives one directive at a time as JSON and returns the
    /// directives replacing it (none, itself, or several), e.g. to modify **single** directives
    Mapper,

    /// the plugin exports `router`, which handles the HTTP requests to `/api/plugins/{name}` and the
    /// paths below it, e.g. to serve a custom report page or its data. It receives the request as
    /// JSON and returns the response as JSON
    Router,

    /// a type this version of zhang does not know, e.g. one added by a newer zhang.
    /// A plugin declaring it still loads; the host ignores the type with a warning.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod test {
    use super::PluginType;

    #[test]
    fn should_read_every_known_type_and_fall_back_to_unknown() {
        let types: Vec<PluginType> = serde_json::from_str(r#"["Processor", "Mapper", "Router", "Reporter"]"#).unwrap();
        assert_eq!(types, vec![PluginType::Processor, PluginType::Mapper, PluginType::Router, PluginType::Unknown]);
        assert_eq!(serde_json::to_string(&PluginType::Router).unwrap(), r#""Router""#);
    }
}
