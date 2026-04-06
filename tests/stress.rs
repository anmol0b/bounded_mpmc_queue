use bounded_mpmc_queue::queue::blocking::BlockingQueue;
use bounded_mpmc_queue::queue::lockfree::LockFreeQueue;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

#[test]
fn no_items_lost_under_heavy_contention_lockfree() {
    let queue = Arc::new(LockFreeQueue::new(1024));
    let mut handles = vec![];
    for _ in 0..16 {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for i in 0..1000 {
                q.push(i);
            }
        }));
    }
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..16 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(count.load(Ordering::Relaxed), 16000);
}

#[test]
fn no_items_lost_under_heavy_contention_blocking() {
    let queue = Arc::new(BlockingQueue::new(1024));
    let mut handles = vec![];
    for _ in 0..16 {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for i in 0..1000 {
                q.push(i);
            }
        }));
    }
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..16 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(count.load(Ordering::Relaxed), 16000);
}
