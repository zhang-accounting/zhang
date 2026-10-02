use std::collections::HashSet;

use axum::extract::State;
use gotcha::api;
use itertools::Itertools;
use zhang_core::plugin::PluginType;

use crate::response::{Nullable, PluginCapabilitiesEntity, PluginEntity, PluginTypeEntity, ResponseWrapper};
use crate::routes::plugin_router::route_of;
use crate::state::SharedLedger;
use crate::ApiResult;

/// The loaded plugins in declaration order, which is the order they run in. A router plugin
/// also lists the route it serves.
#[api(group = "plugin")]
pub async fn plugin_list(ledger: State<SharedLedger>) -> ApiResult<Vec<PluginEntity>> {
    let ledger = ledger.read().await;

    // the first router plugin of a name serves its route
    let mut served = HashSet::new();
    let ret = ledger
        .plugins
        .ordered
        .iter()
        .map(|(plugin, plugin_types)| PluginEntity {
            name: plugin.name.clone(),
            version: plugin.version.clone(),
            plugin_type: plugin_types.iter().filter_map(PluginTypeEntity::from_core).collect(),
            capabilities: PluginCapabilitiesEntity {
                allowed_hosts: plugin.capabilities().allowed_hosts.clone(),
            },
            route: Nullable((plugin_types.contains(&PluginType::Router) && served.insert(plugin.name.as_str())).then(|| route_of(&plugin.name))),
        })
        .collect_vec();
    ResponseWrapper::json(ret)
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::plugin_list;
    use crate::state::SharedLedger;

    /// the WAT fixtures of zhang-core's plugin tests, see `zhang-core/tests/wasm_plugins.rs`
    const FIXTURES: [(&str, &str); 3] = [
        ("echo.wat", include_str!("../../../zhang-core/tests/plugins/echo.wat")),
        ("router.wat", include_str!("../../../zhang-core/tests/plugins/router.wat")),
        ("router_processor.wat", include_str!("../../../zhang-core/tests/plugins/router_processor.wat")),
    ];

    /// A scratch ledger directory under the system temp dir, removed on drop.
    struct ScratchDir(PathBuf);

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[tokio::test]
    async fn plugins_are_listed_in_declaration_order_with_their_capabilities_and_routes() {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-plugin-list-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0).unwrap();
        for (fixture, content) in FIXTURES {
            std::fs::write(dir.0.join(fixture), content).unwrap();
        }
        // a local ledger resolves a module against the working directory, so declare them by absolute path
        let module = |fixture: &str| dir.0.join(fixture).display().to_string();
        let content = format!(
            "option \"features.plugin\" \"true\"\n\
             plugin \"{router}\"\n\
             plugin \"{echo}\"\n  allowed_hosts: \"api.frankfurter.dev\"\n  allowed_hosts: \"api.example.com\"\n\
             plugin \"{echo}\"\n\
             plugin \"{router}\"\n\
             plugin \"{router_processor}\"\n\
             1970-01-01 open Assets:Cash\n",
            router = module("router.wat"),
            echo = module("echo.wat"),
            router_processor = module("router_processor.wat"),
        );
        std::fs::write(dir.0.join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(dir.0.clone(), "main.zhang".to_owned(), source)
            .await
            .unwrap_or_else(|error| panic!("ledger should load: {error}"));

        let response = plugin_list(State(SharedLedger(Arc::new(RwLock::new(ledger))))).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // not sorted or grouped: the same plugin declared twice is listed twice, and only the
        // first router of a name has the route
        let no_hosts = json!({"allowed_hosts": []});
        assert_eq!(
            body,
            json!({"data": [
                {"name": "router-echo", "version": "0.1.0", "plugin_type": ["Router"], "capabilities": no_hosts, "route": "/api/plugins/router-echo"},
                {"name": "echo", "version": "0.1.0", "plugin_type": ["Processor"], "capabilities": {"allowed_hosts": ["api.frankfurter.dev", "api.example.com"]}, "route": null},
                {"name": "echo", "version": "0.1.0", "plugin_type": ["Processor"], "capabilities": no_hosts, "route": null},
                {"name": "router-echo", "version": "0.1.0", "plugin_type": ["Router"], "capabilities": no_hosts, "route": null},
                {"name": "router-processor", "version": "0.1.0", "plugin_type": ["Router", "Processor"], "capabilities": no_hosts, "route": "/api/plugins/router-processor"},
            ]})
        );
    }
}
