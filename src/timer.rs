use futures::Stream;
use tokio_util::time::DelayQueue;
use tokio_util::time::delay_queue::Key;
use std::collections::HashMap;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::pin::Pin;
use std::time::{Duration, Instant};

const NUM_LEVELS: usize = 6;
const MAX_DURATION_UNSIGNED: u64 = (1 << (6 * NUM_LEVELS)) - 1;
const MAX_DURATION_OBJ_STD: Duration = Duration::from_millis(MAX_DURATION_UNSIGNED-5000);

/// What we should do when an item is removed from the delay queue
pub enum ItemHandler {
    /// Call `cb` with `args`
    Call {
        cb: v8::Global<v8::Function>,
        args: Vec<v8::Global<v8::Value>>,
    },
    /// Custom callback function in rust
    RustCall {
        cb: Box<dyn Fn(&mut v8::PinScope, RawItem)>
    }
}

impl ItemHandler {
    pub(super) fn handle<'s>(&self, scope: &mut v8::PinScope<'s, '_>, raw: RawItem) {
        match self {
            Self::Call { cb, args } => {
                let cb = v8::Local::new(scope, cb);
                let args = args.iter().map(|x| v8::Local::new(scope, x)).collect::<Vec<_>>();
                let recv = v8::undefined(scope).into();

                v8::tc_scope!(let scope, scope); // ensure we run everything from here in a try-catch scope
                cb.call(scope, recv, &args);
            },
            Self::RustCall { cb } => (cb)(scope, raw)
        }
    }
}

pub type QueueStreamKey = u64;

#[derive(Debug)]
pub struct RawItem {
    pub final_expiry: Instant,
    pub key: QueueStreamKey,
    // item metadata not used directly by QueueStream
    pub delay: Duration,
    // whether or not to repeat the interval
    pub repeat: bool,
}

/// Item stored and expired from the QueueStream
pub struct Item {
    pub raw: RawItem,
    // item metadata not used directly by QueueStream
    pub handler: Rc<ItemHandler>
}

/// A QueueStream provides an abstraction over tokio_util's DelayQueue with better cancellation support + stream support
pub struct QueueStream {
    queue: DelayQueue<Item>,
    keys: HashMap<QueueStreamKey, Key>,
    waiting_add: Option<Waker>,
    last_key: QueueStreamKey
}

impl QueueStream {
    /// Create a new QueueStream
    pub fn new() -> Self {
        Self { queue: DelayQueue::new(), keys: HashMap::new(), waiting_add: None, last_key: 0 }
    }

    /// Inserts a item handler with the given delay and returns a handle that can be used to cancel it
    pub fn add(&mut self, handler: ItemHandler, delay: Duration, repeat: bool) -> QueueStreamKey {
        let key = self.last_key;
        self.last_key += 1;

        // Add key
        let final_expiry = Instant::now() + delay;
        let safe_delay = Self::get_safe_delay(delay);

        let dkey = self.queue.insert(Item { raw: RawItem { final_expiry, delay, key, repeat }, handler: handler.into() }, safe_delay);
        self.keys.insert(key, dkey);  // Store the key in the cell for later retrieval/cancellation
        if let Some(waker) = self.waiting_add.take() {
            waker.wake();
        }

        key
    }

    /// Inserts a item handler with the given delay and key/handle id
    /// 
    /// Can be used to requeue item handlers while preserving key (RepeatCall uses this for example)
    fn reinsert(&mut self, key: QueueStreamKey, handler: Rc<ItemHandler>, delay: Duration, repeat: bool) {
        let final_expiry = Instant::now() + delay;
        let safe_delay = Self::get_safe_delay(delay);

        let dkey = self.queue.insert(Item { raw: RawItem { final_expiry, delay, key, repeat }, handler }, safe_delay);
        self.keys.insert(key, dkey);  // Store the key in the cell for later retrieval/cancellation
    }

    /// Cancels a key within the queue stream
    pub fn cancel(&mut self, key: QueueStreamKey) -> Option<Item> {
        let dkey = self.keys.remove(&key)?;
        self.queue.try_remove(&dkey).map(|entry| entry.into_inner())
    }

    /// Returns safe delay for delay queue beyond which we need to queue for longer and keep requeueing
    fn get_safe_delay(delay: Duration) -> Duration {
        if delay > MAX_DURATION_OBJ_STD { MAX_DURATION_OBJ_STD } else { delay }
    }

    /// Clears the QueueStream
    pub fn clear(&mut self) {
        self.queue.clear();
        self.keys.clear();
    }

    /// Length of the QueueStream
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

impl Stream for QueueStream {
    type Item = Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {        
        loop {
            match self.queue.poll_expired(cx) {
                Poll::Ready(Some(item)) => {
                    let item = item.into_inner();
                    let now = Instant::now();

                    if now < item.raw.final_expiry {
                        // If we woke up too early, reinsert with the remaining time
                        // and keep looping
                        //
                        // The loop will hit poll_expired again, which registers the waker 
                        // for this new item (or the next earliest item).
                        let remaining = item.raw.final_expiry - now;
                        let safe_delay = Self::get_safe_delay(remaining);

                        // update key in key map
                        let old_key = item.raw.key;
                        if self.keys.contains_key(&old_key) {
                            let new_key = self.queue.insert(item, safe_delay);
                            self.keys.insert(old_key, new_key);
                        }
                        continue;
                    } else {
                        if item.raw.repeat {
                            // We need to readd here before returning to ensure we repeat regardless of if handle() is called or not etc.
                            self.reinsert(item.raw.key, item.handler.clone(), item.raw.delay, item.raw.repeat);
                        } else {
                            // We've actually expired here, return the value
                            self.keys.remove(&item.raw.key);
                        }

                        return Poll::Ready(Some(item));
                    }
                },
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => { 
                    // We want to wait for new items to be added
                    // Store the waker so `add` can wake us up later
                    self.waiting_add = Some(cx.waker().clone());
                    return Poll::Pending
                }
            }
        }
    }
}
