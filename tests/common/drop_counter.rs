//! A value that records its construction and destruction, to prove the queues
//! neither leak nor double-drop.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

#[derive(Debug, Default)]
pub struct DropStats {
    created: AtomicUsize,
    dropped: AtomicUsize,
}

impl DropStats {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn created(&self) -> usize {
        self.created.load(SeqCst)
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(SeqCst)
    }

    pub fn live(&self) -> usize {
        self.created() - self.dropped()
    }
}

/// Holds a heap allocation so Miri reports a double drop as a double free.
#[derive(Debug)]
pub struct Tracked {
    pub id: u64,
    _heap: Box<u64>,
    stats: Arc<DropStats>,
}

impl Tracked {
    pub fn new(id: u64, stats: &Arc<DropStats>) -> Self {
        stats.created.fetch_add(1, SeqCst);
        Self {
            id,
            _heap: Box::new(id),
            stats: Arc::clone(stats),
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        let before = self.stats.dropped.fetch_add(1, SeqCst);
        assert!(
            before < self.stats.created.load(SeqCst),
            "more drops than constructions"
        );
    }
}
