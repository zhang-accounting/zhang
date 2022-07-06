use itertools::Itertools;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

#[derive(Clone, Debug)]
pub(crate) struct LatestMap<Key: Eq + Hash + Clone + Ord, Value> {
    pub(crate) data: HashMap<Key, Value>,
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
            Err(gt_index) => {
                if gt_index == 0 {
                    return None;
                }
                sorted_keys[gt_index - 1]
            }
        };
        self.data.get(target_key)
    }
    pub fn get_last(&self) -> Option<&Value> {
        let sorted_keys = self.date_index.iter().collect_vec();
        sorted_keys.last().and_then(|key| self.data.get(key))
    }
    pub fn get_last_with_key(&self) -> Option<(&Key, &Value)> {
        let sorted_keys = self.date_index.iter().collect_vec();
        sorted_keys.last().and_then(|&key| self.data.get(key).map(|v| (key, v)))
    }

    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        self.data.get_mut(key)
    }
    pub fn contains_key(&self, key: &Key) -> bool {
        self.date_index.contains(key)
    }
}

#[cfg(test)]
mod test {
    use crate::core::utils::latest_map::LatestMap;

    #[test]
    fn should_insert() {
        let mut map: LatestMap<i32, i32> = LatestMap::default();
        map.insert(1, 2);
        assert!(map.date_index.contains(&1));
        assert_eq!(map.data.get(&1), Some(&2));
    }
    #[test]
    fn should_contains_key() {
        let mut map: LatestMap<i32, i32> = LatestMap::default();
        map.insert(1, 2);
        assert!(map.contains_key(&1));
        assert!(!map.contains_key(&2));
    }

    #[test]
    fn should_get_mut() {
        let mut map: LatestMap<i32, i32> = LatestMap::default();
        map.insert(1, 2);
        let value = map.get_mut(&1).unwrap();
        *value = 3;

        assert_eq!(map.data.get(&1), Some(&3));
    }

    #[test]
    fn should_get_latest() {
        let mut map: LatestMap<i32, i32> = LatestMap::default();
        map.insert(1, 2);
        map.insert(10, 20);
        map.insert(20, 40);
        map.insert(50, 100);

        assert_eq!(map.get_latest(&0), None);
        assert_eq!(map.get_latest(&1), Some(&2));
        assert_eq!(map.get_latest(&3), Some(&2));
        assert_eq!(map.get_latest(&20), Some(&40));
        assert_eq!(map.get_latest(&24), Some(&40));
        assert_eq!(map.get_latest(&1000), Some(&100));
    }
    #[test]
    fn should_work_given_map_is_empty() {
        let map: LatestMap<i32, i32> = LatestMap::default();
        assert_eq!(map.get_latest(&0), None);
        assert_eq!(map.get_latest(&1), None);
        assert_eq!(map.get_latest(&3), None);
    }
}
