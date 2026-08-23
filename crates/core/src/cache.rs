//! Bounded LRU cache for rendered pages.
//!
//! The cache keeps at most `capacity` entries (default 3: pages N-1, N, N+1).
//! It is never unbounded. Entries are keyed by document, page and render
//! target dimensions.

use std::collections::{HashMap, VecDeque};

use docbunker_renderer_api::RenderedPage;

const DEFAULT_CAPACITY: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    document: u64,
    page: u32,
    width: u32,
    height: u32,
}

/// Small LRU cache with a fixed capacity.
#[derive(Debug)]
pub struct PageCache {
    capacity: usize,
    order: VecDeque<CacheKey>,
    entries: HashMap<CacheKey, RenderedPage>,
}

impl PageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// Returns a copy of the cached page, if present (promoting it to MRU).
    ///
    /// Copying is intentional for the MVP (the caller owns the buffer); shared
    /// memory / ring buffers are a later optimization.
    pub fn get(
        &mut self,
        document: u64,
        page: u32,
        width: u32,
        height: u32,
    ) -> Option<RenderedPage> {
        let key = CacheKey {
            document,
            page,
            width,
            height,
        };
        if !self.entries.contains_key(&key) {
            return None;
        }
        self.touch(key);
        self.entries.get(&key).cloned()
    }

    /// Insert (or refresh) a cached page.
    pub fn put(
        &mut self,
        document: u64,
        page: u32,
        width: u32,
        height: u32,
        rendered: RenderedPage,
    ) {
        let key = CacheKey {
            document,
            page,
            width,
            height,
        };
        if let Some(existing) = self.entries.get_mut(&key) {
            *existing = rendered;
            self.touch(key);
            return;
        }
        self.entries.insert(key, rendered);
        self.order.push_front(key);
        self.evict();
    }

    /// Drop every entry belonging to `document` (on close).
    pub fn remove_document(&mut self, document: u64) {
        self.order.retain(|k| k.document != document);
        self.entries.retain(|k, _| k.document != document);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn touch(&mut self, key: CacheKey) {
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
            self.order.push_front(key);
        }
    }

    fn evict(&mut self) {
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_back() {
                self.entries.remove(&oldest);
            }
        }
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: u32, height: u32) -> RenderedPage {
        let bytes = vec![0u8; (width * height * 4) as usize];
        RenderedPage {
            width,
            height,
            stride: width * 4,
            pixel_format: docbunker_renderer_api::PixelFormat::Rgba8888,
            bytes,
        }
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut cache = PageCache::new(3);
        cache.put(1, 0, 100, 100, page(100, 100));
        cache.put(1, 1, 100, 100, page(100, 100));
        cache.put(1, 2, 100, 100, page(100, 100));
        assert_eq!(cache.len(), 3);

        cache.put(1, 3, 100, 100, page(100, 100));
        assert_eq!(cache.len(), 3);
        assert!(cache.get(1, 0, 100, 100).is_none());
        assert!(cache.get(1, 1, 100, 100).is_some());
        assert!(cache.get(1, 3, 100, 100).is_some());
    }

    #[test]
    fn refresh_moves_to_front() {
        let mut cache = PageCache::new(2);
        cache.put(1, 0, 100, 100, page(100, 100));
        cache.put(1, 1, 100, 100, page(100, 100));
        // touch page 0
        assert!(cache.get(1, 0, 100, 100).is_some());
        cache.put(1, 2, 100, 100, page(100, 100));
        // page 1 (not 0) is evicted
        assert!(cache.get(1, 1, 100, 100).is_none());
        assert!(cache.get(1, 0, 100, 100).is_some());
    }

    #[test]
    fn remove_document_clears_entries() {
        let mut cache = PageCache::new(3);
        cache.put(1, 0, 100, 100, page(100, 100));
        cache.put(2, 0, 100, 100, page(100, 100));
        cache.remove_document(1);
        assert!(cache.get(1, 0, 100, 100).is_none());
        assert!(cache.get(2, 0, 100, 100).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn capacity_is_never_zero() {
        let mut cache = PageCache::new(0);
        cache.put(1, 0, 10, 10, page(10, 10));
        assert!(cache.get(1, 0, 10, 10).is_some());
    }

    #[test]
    fn keyed_by_target_dimensions() {
        let mut cache = PageCache::new(3);
        cache.put(1, 0, 100, 100, page(100, 100));
        assert!(cache.get(1, 0, 100, 100).is_some());
        assert!(cache.get(1, 0, 200, 200).is_none());
    }
}
