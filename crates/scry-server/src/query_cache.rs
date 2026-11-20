//! Bounded caches for the model round trips of a query. A query vector
//! depends only on the embedder and the HyDE setting, and a rerank only on
//! the query and the documents scored, all fixed for a server's lifetime,
//! so a repeat query skips the chat, embedding, and rerank calls.

use std::borrow::Borrow;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

pub struct QueryCache<K = String, V = Vec<f32>> {
    capacity: usize,
    inner: Mutex<Entries<K, V>>,
}

struct Entries<K, V> {
    values: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Hash + Eq + Clone, V: Clone> QueryCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(Entries {
                values: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner
            .lock()
            .expect("query cache")
            .values
            .get(key)
            .cloned()
    }

    pub fn insert(&self, key: K, value: V) {
        let mut entries = self.inner.lock().expect("query cache");
        if entries.values.insert(key.clone(), value).is_none() {
            entries.order.push_back(key);
        }
        while entries.order.len() > self.capacity {
            let oldest = entries.order.pop_front().expect("non-empty");
            entries.values.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_inserted_vectors_and_evicts_oldest() {
        let cache: QueryCache<&str> = QueryCache::new(2);
        cache.insert("a", vec![1.0]);
        cache.insert("b", vec![2.0]);
        cache.insert("c", vec![3.0]);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(vec![2.0]));
        assert_eq!(cache.get("c"), Some(vec![3.0]));
    }

    #[test]
    fn reinserting_a_key_does_not_grow_the_order() {
        let cache: QueryCache<&str> = QueryCache::new(2);
        cache.insert("a", vec![1.0]);
        cache.insert("a", vec![1.5]);
        cache.insert("b", vec![2.0]);
        assert_eq!(cache.get("a"), Some(vec![1.5]));
        assert_eq!(cache.get("b"), Some(vec![2.0]));
    }
}
