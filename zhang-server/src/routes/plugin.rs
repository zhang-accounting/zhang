use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::PluginType;

use crate::response::{PluginResponse, ResponseWrapper};
use crate::ApiResult;

pub async fn plugin_list(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<PluginResponse>> {
    let store = ledger.read().await;

    let mut grouped_plugins: HashMap<(String, String), Vec<PluginType>> = HashMap::default();

    for (plugin, plugin_type) in store
        .plugins
        .processors
        .iter()
        .map(|it| (it, PluginType::Processor))
        .chain(store.plugins.mappers.iter().map(|it| (it, PluginType::Mapper)))
        .chain(store.plugins.routers.iter().map(|it| (it, PluginType::Router)))
    {
        grouped_plugins
            .entry((plugin.name.to_owned(), plugin.version.to_owned()))
            .or_default()
            .push(plugin_type);
    }

    let ret = grouped_plugins
        .into_iter()
        .map(|(meta, plugin_type)| PluginResponse {
            name: meta.0,
            version: meta.1,
            plugin_type,
        })
        .collect_vec();
    ResponseWrapper::json(ret)
}
