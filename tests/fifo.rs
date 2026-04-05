use bounded_mpmc_queue::queue::blocking::BlockingQueue;

#[test]
fn fifo_ordering_is_preserved_under_sequential_push_pop() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    queue.push(2);
    queue.push(3);
    assert_eq!(queue.pop(), 1);
    assert_eq!(queue.pop(), 2);
    assert_eq!(queue.pop(), 3);
}

#[test]
fn fifo_ordering_is_preserved_at_capacity_boundary() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    queue.push(2);
    queue.push(3);
    queue.push(4);
    assert_eq!(queue.pop(), 1);
    assert_eq!(queue.pop(), 2);
    assert_eq!(queue.pop(), 3);
    assert_eq!(queue.pop(), 4);
}

#[test]
fn fifo_ordering_is_preserved_with_interleaved_operations() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    queue.push(2);
    assert_eq!(queue.pop(), 1);
    queue.push(3);
    queue.push(4);
    assert_eq!(queue.pop(), 2);
    assert_eq!(queue.pop(), 3);
    assert_eq!(queue.pop(), 4);
}

#[test]
fn fifo_ordering_wraps_correctly_around_ring() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    queue.push(2);
    queue.push(3);
    queue.push(4);
    assert_eq!(queue.pop(), 1);
    assert_eq!(queue.pop(), 2);
    queue.push(5);
    queue.push(6);
    assert_eq!(queue.pop(), 3);
    assert_eq!(queue.pop(), 4);
    assert_eq!(queue.pop(), 5);
    assert_eq!(queue.pop(), 6);
}

#[test]
fn fifo_single_item_push_pop() {
    let queue = BlockingQueue::new(4);
    queue.push(1);
    assert_eq!(queue.pop(), 1);
}
