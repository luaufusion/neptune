use futures::{Stream, StreamExt};
use parking_lot::RwLock;
use tokio_util::time::DelayQueue;
use std::sync::{Weak, Arc};
use std::task::{Context, Poll, Waker};
use std::pin::Pin;
use std::time::{Duration, Instant};

const MAX_TIMEOUT: Duration = Duration::from_secs(7);

const NUM_LEVELS: usize = 6;
const MAX_DURATION_UNSIGNED: u64 = (1 << (6 * NUM_LEVELS)) - 1;
const MAX_DURATION_OBJ_STD: Duration = Duration::from_millis(MAX_DURATION_UNSIGNED-5000);

// The key may change if the item is reinserted, so we use a Cell to allow mutability
type SharedKey = Arc<RwLock<Option<tokio_util::time::delay_queue::Key>>>;
type ItemValue = u64;

#[derive(Clone)]
struct Item {
    value: ItemValue,
    final_expiry: std::time::Instant,
    key: SharedKey
}

pub struct KeyHandle {
    queue: Weak<RwLock<DelayQueue<Item>>>,
    key: SharedKey,
}

impl KeyHandle {
    fn cancel(&self) -> Result<(bool, u64), crate::Error> {
        let Some(queue) = self.queue.upgrade() else {
            return Err("Delay channel has been dropped".into());
        };

        let Some(key) = *self.key.try_read().ok_or_else(|| "Cannot cancel while key is being updated!")? else {
            return Ok((false, 0)); // Already removed
        };


        let val = match queue.try_write().ok_or_else(|| "Cannot cancel while DelayQueue.next() is being called!")?.try_remove(&key) {
            Some(val) => {
                *self.key.write() = None; // Clear the key since it's been removed
                Ok((true, val.into_inner().value))
            },
            None => Ok((false, 0)),
        };

        val
    }

    fn is_eq(&self, other: &Self) -> bool {
        *self.key.read() == *other.key.read() && Weak::ptr_eq(&self.queue, &other.queue)
    }
}

/// A delay channel that can be used to return a value after a delay/expiration
pub struct DelayChannel {
    queue: Arc<RwLock<DelayQueue<Item>>>,
    waiting_add: Arc<RwLock<Option<Waker>>>, // used to wake up the stream when a new item is added
}

impl DelayChannel {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(RwLock::new(DelayQueue::new())),
            waiting_add: Arc::new(RwLock::new(None)),
        }
    }

    fn get_safe_delay(delay: Duration) -> Duration {
        if delay > MAX_DURATION_OBJ_STD { MAX_DURATION_OBJ_STD } else { delay }
    }
    
    /// Inserts a value into the delay channel with the given delay
    /// and returns a handle that can be used to cancel it
    pub fn add(&self, value: u64, delay: Duration) -> Result<KeyHandle, crate::Error> {
        let final_expiry = Instant::now() + delay;
        let safe_delay = Self::get_safe_delay(delay);
        let key_cell = Arc::new(RwLock::new(None));
        let key = self.queue.write().insert(Item { value, final_expiry, key: key_cell.clone() }, safe_delay);
        *key_cell.write() = Some(key); // Store the key in the cell for later retrieval
        if let Some(waker) = self.waiting_add.write().take() {
            waker.wake();
        }
        Ok(KeyHandle { key: key_cell, queue: Arc::downgrade(&self.queue) })
    }

    pub async fn next(&self) -> Result<u64, crate::Error> {
        let mut stream = QueueStream {
            queue: self.queue.clone(),
            waiting_add: self.waiting_add.clone(),
        };

        // Attempt to get the next expired item
        match StreamExt::next(&mut stream).await {
            Some(value) => return Ok(value),
            None => {
                // This should never happen, but just in case
                return Err("Delay channel closed unexpectedly".into());
            }
        }
    }

    pub fn clear(&self) {
        self.queue.write().clear()
    }

    pub fn len(&self) -> usize {
        self.queue.read().len()
    }
}

struct QueueStream {
    queue: Arc<RwLock<DelayQueue<Item>>>,
    waiting_add: Arc<RwLock<Option<Waker>>>,
}

impl Stream for QueueStream {
    type Item = u64;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut queue = self.queue.write();
        
        loop {
            match queue.poll_expired(cx) {
                Poll::Ready(Some(item)) => {
                    let item = item.into_inner();
                    let now = Instant::now();

                    if now < item.final_expiry {
                        // If we woke up too early, reinsert with the remaining time
                        // and keep looping
                        //
                        // The loop will hit poll_expired again, which registers the waker 
                        // for this new item (or the next earliest item).
                        let remaining = item.final_expiry - now;
                        let safe_delay = DelayChannel::get_safe_delay(remaining);
                        let new_key = queue.insert(item.clone(), safe_delay);
                        let key_cell = item.key.clone();
                        *key_cell.write() = Some(new_key); // Update the key in the cell
                        continue;
                    } else {
                        // We've actually expired here, return the value
                        let key_cell = item.key.clone();
                        *key_cell.write() = None; // Clear the key since it's been removed
                        return Poll::Ready(Some(item.value));
                    }
                },
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => { 
                    // We want to wait for new items to be added
                    // Store the waker so `add` can wake us up later
                    *self.waiting_add.write() = Some(cx.waker().clone());
                    return Poll::Pending
                }
            }
        }
    }
}
