use std::collections::HashMap;

pub struct Store {
    pub(crate) options: HashMap<String, String>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            options: HashMap::default(),
        }
    }
}