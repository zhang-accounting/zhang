use std::collections::HashMap;

use axum::extract::State;
use gotcha::api;
use itertools::Itertools;
use zhang_core::plugin::PluginType;

use crate::response::{PluginEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::ApiResult;

#[api(group = "plugin")]
pub async fn plugin_list(ledger: State<SharedLedger>) -> ApiResult<Vec<PluginEntity>> {
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
        .map(|(meta, plugin_type)| PluginEntity {
            name: meta.0,
            version: meta.1,
            plugin_type: plugin_type.into_iter().map(|it| it.into()).collect_vec(),
        })
        .collect_vec();
    ResponseWrapper::json(ret)
}
