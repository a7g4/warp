use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::task::Poll;
use tokio::sync::mpsc;

/// Marker type for max-heap behavior (higher values = higher priority)
pub struct MaxPriority;

/// Marker type for min-heap behavior (lower values = higher priority)  
pub struct MinPriority;

/// Trait for configuring priority ordering
pub trait PriorityOrdering {
    const REVERSE: bool;
}

impl PriorityOrdering for MaxPriority {
    const REVERSE: bool = false; // Normal ordering (max-heap)
}

impl PriorityOrdering for MinPriority {
    const REVERSE: bool = true; // Reverse ordering (min-heap)
}

/// Internal wrapper for items in the priority queue
#[derive(Debug)]
struct PriorityItem<T, O> {
    item: T,
    sequence: u64,
    _ordering: std::marker::PhantomData<O>,
}

impl<T, O> PriorityItem<T, O> {
    #[inline]
    fn new(item: T, sequence: u64) -> Self {
        Self {
            item,
            sequence,
            _ordering: std::marker::PhantomData,
        }
    }
}

impl<T, O> PartialEq for PriorityItem<T, O>
where
    T: Ord,
{
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.item == other.item && self.sequence == other.sequence
    }
}

impl<T, O> Eq for PriorityItem<T, O> where T: Ord {}

impl<T, O> PartialOrd for PriorityItem<T, O>
where
    T: Ord,
    O: PriorityOrdering,
{
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Use the same ordering logic as Ord implementation
        let item_cmp = if O::REVERSE {
            other.item.cmp(&self.item)
        } else {
            self.item.cmp(&other.item)
        };

        Some(match item_cmp {
            Ordering::Equal => other.sequence.cmp(&self.sequence), // Earlier sequence first
            other => other,
        })
    }
}

impl<T, O> Ord for PriorityItem<T, O>
where
    T: Ord,
    O: PriorityOrdering,
{
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        // Use const generics for compile-time optimization
        let item_cmp = if O::REVERSE {
            other.item.cmp(&self.item)
        } else {
            self.item.cmp(&other.item)
        };

        match item_cmp {
            Ordering::Equal => other.sequence.cmp(&self.sequence), // Earlier sequence first
            other => other,
        }
    }
}

/// Sender half of the priority queue - wraps tokio::sync::mpsc::UnboundedSender
pub struct Sender<T> {
    inner: mpsc::UnboundedSender<T>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Sender<T> {
    /// Send an item to the priority queue (infallible for unbounded queue)
    #[inline]
    pub fn send(&self, item: T) {
        // This is infallible for unbounded channels, so we ignore the result
        let _ = self.inner.send(item);
    }
}

/// Receiver half of the priority queue - maintains a BinaryHeap for priority ordering
pub struct Receiver<T, O> {
    inner: mpsc::UnboundedReceiver<T>,
    priority_queue: BinaryHeap<PriorityItem<T, O>>,
    sequence_counter: u64,
    _ordering: std::marker::PhantomData<O>,
}

impl<T, O> Receiver<T, O>
where
    T: Ord,
    O: PriorityOrdering,
{
    /// Receive the next highest priority item
    #[inline]
    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(|cx| {
            // First, drain any available messages from the channel into the priority queue
            let len = self.inner.len();
            let mut buffer = Vec::with_capacity(len);
            if self.inner.poll_recv_many(cx, &mut buffer, len).is_ready() {
                for item in buffer {
                    let priority_item = PriorityItem::new(item, self.sequence_counter);
                    self.sequence_counter += 1;
                    self.priority_queue.push(priority_item);
                }
            }

            // Now return the next item from the priority queue
            if let Some(priority_item) = self.priority_queue.pop() {
                return Poll::Ready(Some(priority_item.item));
            }

            // Priority queue is empty, poll for new messages
            self.inner.poll_recv(cx)
        })
        .await
    }
}

#[inline]
pub fn unbounded_priority_queue_with_ordering<T, O>() -> (Sender<T>, Receiver<T, O>)
where
    T: Ord,
    O: PriorityOrdering,
{
    let (tx, rx) = mpsc::unbounded_channel();

    let sender = Sender { inner: tx };

    let receiver = Receiver {
        inner: rx,
        priority_queue: BinaryHeap::new(),
        sequence_counter: 0,
        _ordering: std::marker::PhantomData,
    };

    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestMessage {
        id: u32,
        priority: i64,
        data: String,
    }

    impl PartialOrd for TestMessage {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for TestMessage {
        fn cmp(&self, other: &Self) -> Ordering {
            self.priority.cmp(&other.priority)
        }
    }

    #[tokio::test]
    async fn test_basic_priority_ordering() {
        let (tx, mut rx) = unbounded_priority_queue_with_ordering::<TestMessage, MaxPriority>();

        tx.send(TestMessage {
            id: 1,
            priority: 10,
            data: "low".to_string(),
        });
        tx.send(TestMessage {
            id: 2,
            priority: 50,
            data: "high".to_string(),
        });
        tx.send(TestMessage {
            id: 3,
            priority: 30,
            data: "medium".to_string(),
        });

        drop(tx);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.priority, 50);

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(msg2.priority, 30);

        let msg3 = rx.recv().await.unwrap();
        assert_eq!(msg3.priority, 10);

        // No more messages
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_min_priority_ordering() {
        let (tx, mut rx) = unbounded_priority_queue_with_ordering::<TestMessage, MinPriority>();

        tx.send(TestMessage {
            id: 1,
            priority: 10,
            data: "high".to_string(),
        });
        tx.send(TestMessage {
            id: 2,
            priority: 50,
            data: "low".to_string(),
        });
        tx.send(TestMessage {
            id: 3,
            priority: 30,
            data: "medium".to_string(),
        });

        drop(tx);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.priority, 10);

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(msg2.priority, 30);

        let msg3 = rx.recv().await.unwrap();
        assert_eq!(msg3.priority, 50);

        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_fifo_for_equal_priorities() {
        let (tx, mut rx) = unbounded_priority_queue_with_ordering::<TestMessage, MaxPriority>();

        tx.send(TestMessage {
            id: 1,
            priority: 10,
            data: "first".to_string(),
        });
        tx.send(TestMessage {
            id: 2,
            priority: 10,
            data: "second".to_string(),
        });
        tx.send(TestMessage {
            id: 3,
            priority: 10,
            data: "third".to_string(),
        });

        drop(tx);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.id, 1);

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(msg2.id, 2);

        let msg3 = rx.recv().await.unwrap();
        assert_eq!(msg3.id, 3);
    }

    #[tokio::test]
    async fn test_empty_queue_edge_case() {
        let (tx, mut rx) = unbounded_priority_queue_with_ordering::<TestMessage, MaxPriority>();

        let recv_task = tokio::spawn(async move { rx.recv().await });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        tx.send(TestMessage {
            id: 1,
            priority: 10,
            data: "immediate".to_string(),
        });

        let result = recv_task.await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, 1);
    }

    #[tokio::test]
    async fn test_multiple_senders() {
        let (tx, mut rx) = unbounded_priority_queue_with_ordering::<TestMessage, MaxPriority>();

        let tx1 = tx.clone();
        let tx2 = tx.clone();

        tx1.send(TestMessage {
            id: 1,
            priority: 20,
            data: "tx1".to_string(),
        });
        tx2.send(TestMessage {
            id: 2,
            priority: 30,
            data: "tx2".to_string(),
        });
        tx.send(TestMessage {
            id: 3,
            priority: 10,
            data: "tx".to_string(),
        });

        drop(tx);
        drop(tx1);
        drop(tx2);

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.priority, 30);

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(msg2.priority, 20);

        let msg3 = rx.recv().await.unwrap();
        assert_eq!(msg3.priority, 10);
    }
}
