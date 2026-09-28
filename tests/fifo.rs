use bounded_mpmc_queue::BlockingQueue;

#[test]
fn fifo_ordering_is_preserved_under_sequential_push_pop() {
    let queue = BlockingQueue::new(4);
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    queue.push(3).unwrap();
    assert_eq!(queue.pop(), Ok(1));
    assert_eq!(queue.pop(), Ok(2));
    assert_eq!(queue.pop(), Ok(3));
}

#[test]
fn fifo_ordering_is_preserved_at_capacity_boundary() {
    let queue = BlockingQueue::new(4);
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    queue.push(3).unwrap();
    queue.push(4).unwrap();
    assert_eq!(queue.pop(), Ok(1));
    assert_eq!(queue.pop(), Ok(2));
    assert_eq!(queue.pop(), Ok(3));
    assert_eq!(queue.pop(), Ok(4));
}

#[test]
fn fifo_ordering_is_preserved_with_interleaved_operations() {
    let queue = BlockingQueue::new(4);
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    assert_eq!(queue.pop(), Ok(1));
    queue.push(3).unwrap();
    queue.push(4).unwrap();
    assert_eq!(queue.pop(), Ok(2));
    assert_eq!(queue.pop(), Ok(3));
    assert_eq!(queue.pop(), Ok(4));
}

#[test]
fn fifo_ordering_wraps_correctly_around_ring() {
    let queue = BlockingQueue::new(4);
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    queue.push(3).unwrap();
    queue.push(4).unwrap();
    assert_eq!(queue.pop(), Ok(1));
    assert_eq!(queue.pop(), Ok(2));
    queue.push(5).unwrap();
    queue.push(6).unwrap();
    assert_eq!(queue.pop(), Ok(3));
    assert_eq!(queue.pop(), Ok(4));
    assert_eq!(queue.pop(), Ok(5));
    assert_eq!(queue.pop(), Ok(6));
}

#[test]
fn fifo_single_item_push_pop() {
    let queue = BlockingQueue::new(4);
    queue.push(1).unwrap();
    assert_eq!(queue.pop(), Ok(1));
}
