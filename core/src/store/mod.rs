use std::collections::HashMap;
use crate::domains::schemas::{CommodityDomain, MetaDomain};

pub struct Store {
    pub(crate) options: HashMap<String, String>,
    pub(crate) commodities: HashMap<String, CommodityDomain>,
    pub(crate) metas: Vec<MetaDomain>
}

impl Default for Store {
    fn default() -> Self {
        Self {
            options: HashMap::default(),
            commodities: Default::default(),
            metas: vec![],
        }
    }
}