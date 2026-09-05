//! Query vectors keyed by query text. A vector depends only on the
//! embedder and the HyDE setting, both fixed for a server's lifetime, so
//! a repeat query skips the chat and embedding round trips.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub struct QueryCache {
    capacity: usize,
    inner: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    vectors: HashMap<String, Vec<f32>>,
    order: VecDeque<String>,
}

impl QueryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(Entries::default()),
        }
    }

    pub fn get(&self, query: &str) -> Option<Vec<f32>> {
        self.inner
            .lock()
            .expect("query cache")
            .vectors
            .get(query)
            .cloned()
    }

    pub fn insert(&self, query: &str, vector: Vec<f32>) {
        let mut entries = self.inner.lock().expect("query cache");
        if entries.vectors.insert(query.to_string(), vector).is_none() {
            entries.order.push_back(query.to_string());
        }
        while entries.order.len() > self.capacity {
            let oldest = entries.order.pop_front().expect("non-empty");
            entries.vectors.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_inserted_vectors_and_evicts_oldest() {
        let cache = QueryCache::new(2);
        cache.insert("a", vec![1.0]);
        cache.insert("b", vec![2.0]);
        cache.insert("c", vec![3.0]);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(vec![2.0]));
        assert_eq!(cache.get("c"), Some(vec![3.0]));
    }

    #[test]
    fn reinserting_a_key_does_not_grow_the_order() {
        let cache = QueryCache::new(2);
        cache.insert("a", vec![1.0]);
        cache.insert("a", vec![1.5]);
        cache.insert("b", vec![2.0]);
        assert_eq!(cache.get("a"), Some(vec![1.5]));
        assert_eq!(cache.get("b"), Some(vec![2.0]));
    }
}
