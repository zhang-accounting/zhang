use itertools::Itertools;
use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Default)]
pub(crate) struct MultiValueMap<Key: Eq + Hash, Value> {
    inner: HashMap<Key, Vec<Value>>,
}

impl<Key: Hash + Eq, Value> MultiValueMap<Key, Value> {
    pub fn insert(&mut self, key: Key, value: Value) {
        let key_store = self.inner.entry(key).or_insert_with(Vec::new);
        key_store.push(value);
    }

    pub fn get_one(&self, key: &Key) -> Option<&Value> {
        self.inner.get(key).and_then(|store| store.get(0))
    }

    pub fn get_all(&self, key: &Key) -> Vec<&Value> {
        self.inner
            .get(key)
            .map(|it| it.iter().collect_vec())
            .unwrap_or_default()
    }
}
