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

#[cfg(test)]
mod test {
    use crate::core::utils::multi_value_map::MultiValueMap;

    #[test]
    fn should_insert() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        map.insert(1, 2);
        assert!(map.inner.contains_key(&1));
        assert_eq!(map.inner.get(&1).unwrap(), &vec![2i32]);
    }
    #[test]
    fn should_insert_multiple_value() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        map.insert(1, 2);
        map.insert(1, 3);
        assert!(map.inner.contains_key(&1));
        assert_eq!(map.inner.get(&1).unwrap(), &vec![2i32, 3i32]);
    }
    #[test]
    fn should_get_none_given_empty() {
        let map: MultiValueMap<i32, i32> = MultiValueMap::default();
        assert_eq!(map.get_one(&1), None);
        let vec1: Vec<&i32> = vec![];
        assert_eq!(map.get_all(&1), vec1);
    }
    #[test]
    fn should_get_correct_value() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        map.insert(1, 2);
        map.insert(1, 3);
        assert_eq!(map.get_one(&1), Some(&2));
        assert_eq!(map.get_all(&1), vec![&2, &3]);
    }
}
