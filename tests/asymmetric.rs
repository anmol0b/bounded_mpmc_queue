use bounded_mpmc_queue::queue::lockfree::LockFreeQueue;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

#[test]
fn many_producers_single_consumer() {
    let queue = Arc::new(LockFreeQueue::new(1024));
    let mut producer_handles = vec![];
    for _ in 0..8 {
        let q = Arc::clone(&queue);
        producer_handles.push(thread::spawn(move || {
            for i in 0..100 {
                q.push(i);
            }
        }));
    }
    for handle in producer_handles {
        handle.join().unwrap();
    }
    let mut count = 0;
    while queue.try_pop().is_some() {
        count += 1;
    }

    assert_eq!(count, 800);
}

#[test]
fn single_producer_many_consumers() {
    let queue = Arc::new(LockFreeQueue::new(1024));
    for i in 0..800 {
        queue.push(i);
    }

    let count = Arc::new(AtomicUsize::new(0));
    let mut consumer_handles = vec![];
    for _ in 0..8 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        consumer_handles.push(thread::spawn(move || {
            for _ in 0..100 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for handle in consumer_handles {
        handle.join().unwrap();
    }

    assert_eq!(count.load(Ordering::Relaxed), 800);
}

#[test]
fn more_producers_than_consumers() {
    let queue = Arc::new(LockFreeQueue::new(1024));
    let mut producer_handles = vec![];
    for _ in 0..8 {
        let q = Arc::clone(&queue);
        producer_handles.push(thread::spawn(move || {
            for i in 0..100 {
                q.push(i);
            }
        }));
    }

    for handle in producer_handles {
        handle.join().unwrap();
    }
    let count = Arc::new(AtomicUsize::new(0));
    let mut consumer_handles = vec![];
    for _ in 0..2 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        consumer_handles.push(thread::spawn(move || {
            for _ in 0..400 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for handle in consumer_handles {
        handle.join().unwrap();
    }

    assert_eq!(count.load(Ordering::Relaxed), 800);
}
