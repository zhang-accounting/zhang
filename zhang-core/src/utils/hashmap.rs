use std::collections::HashMap;

pub trait HashMapOfExt<Key, Value> {
    fn of(key: impl Into<Key>, value: impl Into<Value>) -> HashMap<Key, Value>;
    fn of2(key1: impl Into<Key>, value1: impl Into<Value>, key2: impl Into<Key>, value2: impl Into<Value>) -> HashMap<Key, Value>;
    fn of3(
        key1: impl Into<Key>, value1: impl Into<Value>, key2: impl Into<Key>, value2: impl Into<Value>, key3: impl Into<Key>, value3: impl Into<Value>,
    ) -> HashMap<Key, Value>;
}

impl HashMapOfExt<String, String> for HashMap<String, String> {
    fn of(key: impl Into<String>, value: impl Into<String>) -> HashMap<String, String> {
        let mut map = HashMap::default();
        map.insert(key.into(), value.into());
        map
    }

    fn of2(key1: impl Into<String>, value1: impl Into<String>, key2: impl Into<String>, value2: impl Into<String>) -> HashMap<String, String> {
        let mut map = HashMap::default();
        map.insert(key1.into(), value1.into());
        map.insert(key2.into(), value2.into());
        map
    }

    fn of3(
        key1: impl Into<String>, value1: impl Into<String>, key2: impl Into<String>, value2: impl Into<String>, key3: impl Into<String>,
        value3: impl Into<String>,
    ) -> HashMap<String, String> {
        let mut map = HashMap::default();
        map.insert(key1.into(), value1.into());
        map.insert(key2.into(), value2.into());
        map.insert(key3.into(), value3.into());
        map
    }
}
