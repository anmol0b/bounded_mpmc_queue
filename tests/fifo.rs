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
