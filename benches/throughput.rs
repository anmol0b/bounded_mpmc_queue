use bounded_mpmc_queue::queue::blocking::BlockingQueue;
use bounded_mpmc_queue::queue::lockfree::LockFreeQueue;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::sync::Arc;
use std::thread;

fn bench_blocking(c: &mut Criterion, threads: usize, capacity: usize) {
    let mut group = c.benchmark_group("blocking");
    group.bench_with_input(
        BenchmarkId::new(format!("threads_{threads}_cap_{capacity}"), ""),
        &(threads, capacity),
        |b, &(threads, capacity)| {
            b.iter(|| {
                let queue = Arc::new(BlockingQueue::new(capacity));
                let mut handles = vec![];
                for _ in 0..threads {
                    let q = Arc::clone(&queue);
                    handles.push(thread::spawn(move || {
                        for i in 0..100 {
                            q.push(i);
                        }
                    }));
                }
                for _ in 0..threads {
                    let q = Arc::clone(&queue);
                    handles.push(thread::spawn(move || {
                        for _ in 0..100 {
                            q.pop();
                        }
                    }));
                }
                for handle in handles {
                    handle.join().unwrap();
                }
            });
        },
    );
    group.finish();
}

fn bench_lockfree(c: &mut Criterion, threads: usize, capacity: usize) {
    let mut group = c.benchmark_group("lockfree");
    group.bench_with_input(
        BenchmarkId::new(format!("threads_{threads}_cap_{capacity}"), ""),
        &(threads, capacity),
        |b, &(threads, capacity)| {
            b.iter(|| {
                let queue = Arc::new(LockFreeQueue::new(capacity));
                let mut handles = vec![];
                for _ in 0..threads {
                    let q = Arc::clone(&queue);
                    handles.push(thread::spawn(move || {
                        for i in 0..100 {
                            q.push(i);
                        }
                    }));
                }
                for _ in 0..threads {
                    let q = Arc::clone(&queue);
                    handles.push(thread::spawn(move || {
                        for _ in 0..100 {
                            q.pop();
                        }
                    }));
                }
                for handle in handles {
                    handle.join().unwrap();
                }
            });
        },
    );
    group.finish();
}

fn bench_asymmetric(c: &mut Criterion) {
    let mut group = c.benchmark_group("asymmetric");
    group.bench_function("lockfree_8producers_2consumers", |b| {
        b.iter(|| {
            let queue = Arc::new(LockFreeQueue::new(1024));
            let mut handles = vec![];
            for _ in 0..8 {
                let q = Arc::clone(&queue);
                handles.push(thread::spawn(move || {
                    for i in 0..100 {
                        q.push(i);
                    }
                }));
            }
            for _ in 0..2 {
                let q = Arc::clone(&queue);
                handles.push(thread::spawn(move || {
                    for _ in 0..400 {
                        q.pop();
                    }
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
    group.bench_function("lockfree_1producer_8consumers", |b| {
        b.iter(|| {
            let queue = Arc::new(LockFreeQueue::new(1024));
            let mut handles = vec![];
            let q = Arc::clone(&queue);
            handles.push(thread::spawn(move || {
                for i in 0..800 {
                    q.push(i);
                }
            }));
            for _ in 0..8 {
                let q = Arc::clone(&queue);
                handles.push(thread::spawn(move || {
                    for _ in 0..100 {
                        q.pop();
                    }
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
    group.finish();
}

fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");
    for threads in [1, 2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("blocking", threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let queue = Arc::new(BlockingQueue::new(1024));
                    let mut handles = vec![];
                    for _ in 0..threads {
                        let q = Arc::clone(&queue);
                        handles.push(thread::spawn(move || {
                            for i in 0..100 {
                                q.push(i);
                            }
                        }));
                    }
                    for _ in 0..threads {
                        let q = Arc::clone(&queue);
                        handles.push(thread::spawn(move || {
                            for _ in 0..100 {
                                q.pop();
                            }
                        }));
                    }
                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("lockfree", threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let queue = Arc::new(LockFreeQueue::new(1024));
                    let mut handles = vec![];
                    for _ in 0..threads {
                        let q = Arc::clone(&queue);
                        handles.push(thread::spawn(move || {
                            for i in 0..100 {
                                q.push(i);
                            }
                        }));
                    }
                    for _ in 0..threads {
                        let q = Arc::clone(&queue);
                        handles.push(thread::spawn(move || {
                            for _ in 0..100 {
                                q.pop();
                            }
                        }));
                    }
                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
    }
    group.finish();
}

fn throughput_benchmark(c: &mut Criterion) {
    for threads in [1, 2, 4, 8, 16] {
        for capacity in [64, 256, 1024] {
            bench_blocking(c, threads, capacity);
            bench_lockfree(c, threads, capacity);
        }
    }
    bench_asymmetric(c);
    bench_scaling(c);
}
criterion_group!(benches, throughput_benchmark);
criterion_main!(benches);
