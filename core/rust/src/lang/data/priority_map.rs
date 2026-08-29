//! Stable persistent priority map.

use crate::lang::data::{Map, OrderedMap, SortedMap};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICount, IDisplay, IDissoc, IEmpty, IEquality, IFind, IHash,
    ILookup, IMetadata, IObjType, IPeekFirst, IPeekLast, IPersistent, IPopFirst, IPopLast,
    MetaType, ObjType,
};
use std::hash::Hash;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct Standard<K, V> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    priorities: Map<K, V>,
    buckets: SortedMap<V, OrderedMap<K, ()>>,
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> Default for Standard<K, V> {
    fn default() -> Self {
        Self {
            metadata: None,
            priorities: Map::new(),
            buckets: SortedMap::new(),
        }
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> Standard<K, V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.priorities.len()
    }
    pub fn is_empty(&self) -> bool {
        self.priorities.is_empty()
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.priorities.get(key)
    }
    pub fn iter(&self) -> std::vec::IntoIter<(K, V)> {
        let mut out = Vec::with_capacity(self.len());
        for (priority, bucket) in self.buckets.iter() {
            for (key, _) in bucket.iter() {
                out.push((key.clone(), priority.clone()));
            }
        }
        out.into_iter()
    }
    pub fn assoc_value(&self, key: K, priority: V) -> Self {
        if self.get(&key) == Some(&priority) {
            return self.clone();
        }
        let mut buckets = self.buckets.clone();
        if let Some(old) = self.get(&key) {
            if let Some(bucket) = buckets.get(old) {
                let next = bucket.dissoc_value(&key);
                buckets = if next.is_empty() {
                    buckets.dissoc_value(old)
                } else {
                    buckets.assoc_value(old.clone(), next)
                };
            }
        }
        let bucket = buckets
            .get(&priority)
            .cloned()
            .unwrap_or_else(OrderedMap::new)
            .assoc_value(key.clone(), ());
        Self {
            metadata: self.metadata.clone(),
            priorities: self.priorities.assoc_value(key, priority.clone()),
            buckets: buckets.assoc_value(priority, bucket),
        }
    }
    pub fn dissoc_value(&self, key: &K) -> Self {
        let Some(priority) = self.get(key) else {
            return self.clone();
        };
        let mut buckets = self.buckets.clone();
        if let Some(bucket) = buckets.get(priority) {
            let next = bucket.dissoc_value(key);
            buckets = if next.is_empty() {
                buckets.dissoc_value(priority)
            } else {
                buckets.assoc_value(priority.clone(), next)
            };
        }
        Self {
            metadata: self.metadata.clone(),
            priorities: self.priorities.dissoc_value(key),
            buckets,
        }
    }
    pub fn peek_first_entry(&self) -> Option<(K, V)> {
        self.iter().next()
    }
    pub fn peek_last_entry(&self) -> Option<(K, V)> {
        self.iter().last()
    }
    pub fn pop_first_value(&self) -> Self {
        self.peek_first_entry()
            .map(|(k, _)| self.dissoc_value(&k))
            .unwrap_or_else(|| self.clone())
    }
    pub fn pop_last_value(&self) -> Self {
        self.peek_last_entry()
            .map(|(k, _)| self.dissoc_value(&k))
            .unwrap_or_else(|| self.clone())
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> FromIterator<(K, V)> for Standard<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(it: T) -> Self {
        it.into_iter()
            .fold(Self::new(), |m, (k, v)| m.assoc_value(k, v))
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IntoIterator for Standard<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> ICount for Standard<K, V> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IFind<K> for Standard<K, V> {
    type Output = (K, V);
    fn find(&self, key: &K) -> Option<Self::Output> {
        self.priorities
            .find_entry(key)
            .map(|(k, v)| (k.clone(), v.clone()))
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> ILookup<K, V> for Standard<K, V> {
    type Keys = std::vec::IntoIter<K>;
    type Values = std::vec::IntoIter<V>;
    fn keys(&self) -> Self::Keys {
        self.iter().map(|(k, _)| k).collect::<Vec<_>>().into_iter()
    }
    fn vals(&self) -> Self::Values {
        self.iter().map(|(_, v)| v).collect::<Vec<_>>().into_iter()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IAssoc<K, V> for Standard<K, V> {
    type Output = Self;
    fn assoc(&self, k: K, v: V) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IDissoc<K> for Standard<K, V> {
    type Output = Self;
    fn dissoc(&self, k: &K) -> Self {
        self.dissoc_value(k)
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IConj<(K, V)> for Standard<K, V> {
    type Output = Self;
    fn conj(&self, (k, v): (K, V)) -> Self {
        self.assoc_value(k, v)
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IPeekFirst<(K, V)> for Standard<K, V> {
    fn peek_first(&self) -> Option<(K, V)> {
        self.peek_first_entry()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IPeekLast<(K, V)> for Standard<K, V> {
    fn peek_last(&self) -> Option<(K, V)> {
        self.peek_last_entry()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IPopFirst for Standard<K, V> {
    type Output = Self;
    fn pop_first(&self) -> Self {
        self.pop_first_value()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IPopLast for Standard<K, V> {
    type Output = Self;
    fn pop_last(&self) -> Self {
        self.pop_last_value()
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IEmpty for Standard<K, V> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IMetadata for Standard<K, V> {
    type Metadata = Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            metadata,
            ..self.clone()
        }
    }
    fn metatype(&self) -> MetaType {
        MetaType::Map
    }
}
impl<K: Clone + Eq + Hash, V: Clone + Ord> IPersistent for Standard<K, V> {}
impl<K: Clone + Eq + Hash, V: Clone + Ord + PartialEq> IEquality for Standard<K, V> {
    fn equality(&self, other: &Self) -> bool {
        self.len() == other.len() && self.priorities.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}
impl<K: Clone + Eq + Hash + std::fmt::Debug, V: Clone + Ord + std::fmt::Debug> IDisplay
    for Standard<K, V>
{
    fn display(&self) -> String {
        format!(
            "{{{}}}",
            self.iter()
                .map(|(k, v)| format!("{k:?} {v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<K: Clone + Eq + Hash + JavaHash, V: Clone + Ord + Hash + JavaHash> IHash for Standard<K, V> {
    fn hash_calc(&self, t: HashType) -> u64 {
        crate::lang::hash::compose_unordered(
            "MAP",
            self.priorities
                .iter()
                .map(|(k, v)| crate::lang::hash::compose_entry(k.java_hash(t), v.java_hash(t))),
        ) as u64
    }
}
impl<K: Clone + Eq + Hash + std::fmt::Debug, V: Clone + Ord + std::fmt::Debug> IObjType
    for Standard<K, V>
{
    fn obj_type(&self) -> ObjType {
        ObjType::Map
    }
}
impl<K, V> IColl<(K, V)> for Standard<K, V>
where
    K: Clone + Eq + Hash + JavaHash + std::fmt::Debug,
    V: Clone + Ord + PartialEq + Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "{"
    }
    fn end_string(&self) -> &'static str {
        "}"
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn stable_ties_and_priority_updates() {
        let map = Standard::new()
            .assoc_value("a", 2)
            .assoc_value("b", 1)
            .assoc_value("c", 1);
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![("b", 1), ("c", 1), ("a", 2)]
        );
        let moved = map.assoc_value("b", 2);
        assert_eq!(
            moved.iter().collect::<Vec<_>>(),
            vec![("c", 1), ("a", 2), ("b", 2)]
        );
        assert_eq!(map.get(&"b"), Some(&1));
        assert_eq!(moved.peek_last_entry(), Some(("b", 2)));
    }
}
