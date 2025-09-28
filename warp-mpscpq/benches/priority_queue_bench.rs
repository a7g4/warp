use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::cmp::Ordering;
use tokio::runtime::Runtime;
use warp_mpscpq::{unbounded_priority_queue_with_ordering, MaxPriority};

#[derive(Debug, Clone)]
struct BenchMessage {
    id: u64,
    priority: i64,
    data: Vec<u8>,
}

impl PartialEq for BenchMessage {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for BenchMessage {}

impl PartialOrd for BenchMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BenchMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

fn bench_realistic_usage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("realistic_usage");

    let total_messages = 1000;
    let batch_sizes = [1, 2, 4, 8, 16, 32, 64, 128];

    // Regular MPSC with different batch sizes
    for &batch_size in &batch_sizes {
        let bench_name = format!("regular_mpsc_batch_{}", batch_size);
        group.bench_function(&bench_name, |b| {
            b.iter(|| {
                rt.block_on(async {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BenchMessage>();

                    let num_batches = total_messages / batch_size;
                    let mut message_id = 0;

                    for _batch in 0..num_batches {
                        // Send a batch of messages
                        for _i in 0..batch_size {
                            let msg = BenchMessage {
                                id: message_id,
                                priority: (message_id % 100) as i64,
                                data: vec![0u8; 64],
                            };
                            tx.send(msg).unwrap();
                            message_id += 1;
                        }

                        // Receive the batch (up to batch_size messages)
                        let mut batch_received = Vec::new();
                        for _i in 0..batch_size {
                            if let Some(msg) = rx.recv().await {
                                batch_received.push(msg);
                            }
                        }
                        black_box(batch_received);
                    }

                    drop(tx);

                    // Receive any remaining messages
                    let mut remaining = Vec::new();
                    while let Some(msg) = rx.recv().await {
                        remaining.push(msg);
                    }
                    black_box(remaining);
                });
            });
        });
    }

    // Priority MPSC with different batch sizes
    for &batch_size in &batch_sizes {
        let bench_name = format!("priority_mpsc_batch_{}", batch_size);
        group.bench_function(&bench_name, |b| {
            b.iter(|| {
                rt.block_on(async {
                    let (tx, mut rx) = unbounded_priority_queue_with_ordering::<BenchMessage, MaxPriority>();

                    let num_batches = total_messages / batch_size;
                    let mut message_id = 0;

                    for _batch in 0..num_batches {
                        // Send a batch of messages
                        for _i in 0..batch_size {
                            let msg = BenchMessage {
                                id: message_id,
                                priority: (message_id % 100) as i64,
                                data: vec![0u8; 64],
                            };
                            tx.send(msg);
                            message_id += 1;
                        }

                        let mut batch_received = Vec::new();
                        for _i in 0..batch_size {
                            if let Some(msg) = rx.recv().await {
                                batch_received.push(msg);
                            }
                        }
                        black_box(batch_received);
                    }

                    drop(tx);

                    // Receive any remaining messages
                    let mut remaining = Vec::new();
                    while let Some(msg) = rx.recv().await {
                        remaining.push(msg);
                    }
                    black_box(remaining);
                });
            });
        });
    }

    group.finish();
}

fn bench_burst_scenarios(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("burst_scenarios");

    // Test burst sending followed by burst receiving (more realistic for priority queues)
    for &batch_size in &[1, 4, 16, 64] {
        let bench_name = format!("priority_burst_{}", batch_size);
        group.bench_function(&bench_name, |b| {
            b.iter(|| {
                rt.block_on(async {
                    let (tx, mut rx) = unbounded_priority_queue_with_ordering::<BenchMessage, MaxPriority>();

                    let total_messages = 1000;
                    let num_bursts = total_messages / batch_size;

                    for burst in 0..num_bursts {
                        for i in 0..batch_size {
                            let message_id = burst * batch_size + i;
                            let msg = BenchMessage {
                                id: message_id,
                                priority: ((message_id * 7) % 100) as i64,
                                data: vec![0u8; 64],
                            };
                            tx.send(msg);
                        }

                        tokio::task::yield_now().await;

                        // Receive and process the burst
                        let mut burst_received = Vec::new();
                        for _i in 0..batch_size {
                            if let Some(msg) = rx.recv().await {
                                burst_received.push(msg);
                            }
                        }

                        black_box(burst_received);
                    }

                    drop(tx);

                    // Clean up any remaining messages
                    while let Some(msg) = rx.recv().await {
                        black_box(msg);
                    }
                });
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_realistic_usage, bench_burst_scenarios);
criterion_main!(benches);
