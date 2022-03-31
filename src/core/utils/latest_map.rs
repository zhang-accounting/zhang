use itertools::Itertools;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

#[derive(Clone, Debug)]
pub(crate) struct LatestMap<Key: Eq + Hash + Clone + Ord, Value> {
    data: HashMap<Key, Value>,
    date_index: BTreeSet<Key>,
}

impl<Key: Eq + Hash + Clone + Ord, Value> Default for LatestMap<Key, Value> {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            date_index: BTreeSet::new(),
        }
    }
}

impl<Key: Eq + Hash + Clone + Ord, Value> LatestMap<Key, Value> {
    pub fn insert(&mut self, key: Key, value: Value) {
        self.data.insert(key.clone(), value);
        self.date_index.insert(key);
    }
    pub fn get_latest(&self, key: &Key) -> Option<&Value> {
        let sorted_keys = self.date_index.iter().collect_vec();
        let target_key = match sorted_keys.binary_search(&key) {
            Ok(_) => key,
            Err(gt_index) => sorted_keys[gt_index - 1],
        };
        self.data.get(target_key)
    }
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        self.data.get_mut(key)
    }
    pub fn contains_key(&self, key: &Key) -> bool {
        self.date_index.contains(key)
    }
}
