use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiValueMap<Key: Eq + Hash, Value> {
    inner: HashMap<Key, Vec<Value>>,
}

impl<Key: Eq + Hash, Value> Default for MultiValueMap<Key, Value> {
    fn default() -> Self {
        Self { inner: HashMap::new() }
    }
}

impl<Key: Hash + Eq, Value> MultiValueMap<Key, Value> {
    pub fn insert(&mut self, key: Key, value: Value) {
        let key_store = self.inner.entry(key).or_default();
        key_store.push(value);
    }

    pub fn get_one<Q>(&self, key: &Q) -> Option<&Value>
    where
        Key: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.get(key).and_then(|store| store.first())
    }

    pub fn pop_one<Q>(&mut self, key: &Q) -> Option<Value>
    where
        Key: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let value_len = self.inner.get(key).map(|values| values.len());
        match value_len {
            Some(1) => self.inner.remove(key).and_then(|mut values| values.pop()),
            Some(_) => self.inner.get_mut(key).map(|values| values.remove(0)),
            None => None,
        }
    }

    pub fn get_all<Q>(&self, key: &Q) -> Vec<&Value>
    where
        Key: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.get(key).map(|it| it.iter().collect_vec()).unwrap_or_default()
    }
}
impl<Key: Hash + Eq + Clone, Value> MultiValueMap<Key, Value> {
    pub fn get_flatten(self) -> Vec<(Key, Value)> {
        self.inner
            .into_iter()
            .flat_map(|(key, values)| values.into_iter().map(move |value| (key.clone(), value)))
            .collect_vec()
    }
}

impl<Key, Value> FromIterator<(Key, Value)> for MultiValueMap<Key, Value>
where
    Key: Eq + Hash,
{
    fn from_iter<T: IntoIterator<Item = (Key, Value)>>(iter: T) -> Self {
        let mut map = Self::default();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

#[cfg(test)]
mod test {
    use crate::utils::multi_value_map::MultiValueMap;

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
        assert_eq!(map.inner.get(&1i32).unwrap(), &vec![2i32, 3i32]);
    }
    #[test]
    fn should_get_none_given_empty() {
        let map: MultiValueMap<i32, i32> = MultiValueMap::default();
        assert_eq!(map.get_one(&1i32), None);
        let vec1: Vec<&i32> = vec![];
        assert_eq!(map.get_all(&1i32), vec1);
    }
    #[test]
    fn should_get_correct_value() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        map.insert(1, 2);
        map.insert(1, 3);
        assert_eq!(map.get_one(&1i32), Some(&2));
        assert_eq!(map.get_all(&1i32), vec![&2, &3]);
    }

    #[test]
    fn should_pop_with_no_value() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        assert_eq!(map.pop_one(&1i32), None);
    }

    #[test]
    fn should_pop_first_value() {
        let mut map: MultiValueMap<i32, i32> = MultiValueMap::default();
        map.insert(1, 2);
        map.insert(1, 3);
        assert_eq!(map.pop_one(&1i32), Some(2));
        assert_eq!(map.pop_one(&1i32), Some(3));
        assert_eq!(map.pop_one(&1i32), None);
    }
}
