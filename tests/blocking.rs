use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use bounded_mpmc_queue::queue::blocking::BlockingQueue;

#[test]
fn blocking_try_push_returns_err_when_full() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    queue.push(2);
    queue.push(3);
    queue.push(4);
    let result = queue.try_push(5);
    assert!(result.is_err());
}

#[test]
fn blocking_try_pop_returns_none_when_empty() {
    let queue: BlockingQueue<i32> = BlockingQueue::new(4);
    let result = queue.try_pop();
    assert!(result.is_none());
}

#[test]
fn blocking_multiple_producer_single_consumer() {
    let queue = Arc::new(BlockingQueue::new(64));
    let mut handles = vec![];
    for _ in 0..4 {
        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || {
            for i in 0..10 {
                q.push(i);
            }
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let mut count = 0;
    while let Some(_) = queue.try_pop() {
        count += 1;
    }
    assert_eq!(count, 40)
}

#[test]
fn blocking_single_producer_multiple_consumers() {
    let queue = Arc::new(BlockingQueue::new(64));
    let mut handles = vec![];
    for i in 0..40 {
        queue.push(i);
    }
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..4 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(count.load(Ordering::Relaxed), 40);
}

#[test]
fn blocking_multiple_producers_multiple_consumers() {
    let queue = Arc::new(BlockingQueue::new(64));
    let mut producer_handles = vec![];
    for _ in 0..4 {
        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || {
            for i in 0..10 {
                q.push(i);
            }
        });
        producer_handles.push(handle);
    }

    for handle in producer_handles {
        handle.join().unwrap();
    }
    let mut consumer_handles = vec![];
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..4 {
        let q = Arc::clone(&queue);
        let c = Arc::clone(&count);
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                q.pop();
                c.fetch_add(1, Ordering::Relaxed);
            }
        });
        consumer_handles.push(handle);
    }
    for handle in consumer_handles {
        handle.join().unwrap();
    }
    assert_eq!(count.load(Ordering::Relaxed), 40);
}
