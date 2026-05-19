use std::{cell::Cell, collections::HashMap, rc::Rc};

use futures::stream::FuturesUnordered;
use tokio::sync::oneshot;
use crate::{fsw::FilesystemWrapper, module::ModuleRegistry, runtime::{LogMessage, PipedMessage}, timer::QueueStream};
pub type V8Result = Result<(), v8::Global<v8::Value>>;

pub type OpHandlerFut = futures::future::LocalBoxFuture<'static, Box<dyn OpResolver>>;

pub trait OpResolver: 'static {
    fn resolve<'s>(self: Box<Self>, scope: &mut v8::PinScope<'s, '_>);
}

/// Internal v8 isolate state
/// 
/// Internal to Neptune and is subject to change at any time
pub struct IsolateState {
    // scheduler
    pub(super) pending_ops: FuturesUnordered<OpHandlerFut>,

    // modules
    modules: ModuleRegistry,
    module_promise_tracker: HashMap<u64, oneshot::Sender<V8Result>>,
    next_module_promise_tracker_id: u64,
    
    // timer
    pub(super) queue_stream: QueueStream,

    start_time: std::time::Instant,

    // Ensures monotonicity
    last_reported_time: Cell<f64>,

    // embedder pipe
    pub(super) worker_to_embedder_cb: Option<Box<dyn FnMut(PipedMessage)>>,
    pub(super) embedder_to_worker_cb: Option<v8::Global<v8::Function>>,
    pub(super) embedder_log_cb: Option<Rc<dyn Fn(LogMessage)>>
}

impl std::fmt::Debug for IsolateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IsolateState").finish()
    }
}

impl IsolateState {
    /// Creates and attaches a new IsolateState
    /// 
    /// Should not be used outside of runtime
    pub fn attach<'s>(isolate: &mut v8::Isolate, vfs: FilesystemWrapper) {
        isolate.set_slot(Self {
            pending_ops: FuturesUnordered::new(),
            modules: ModuleRegistry::new(vfs),
            module_promise_tracker: HashMap::new(),
            next_module_promise_tracker_id: 0,
            queue_stream: QueueStream::new(),
            start_time: std::time::Instant::now(),
            last_reported_time: Cell::new(0.0),
            worker_to_embedder_cb: None,
            embedder_to_worker_cb: None,
            embedder_log_cb: None
        });
    }

    /// Run function `f` on the underlying IsolateState (immutable) from scope
    #[inline(always)]
    pub fn with<'s, R>(scope: &v8::PinScope<'s, '_, ()>, f: impl FnOnce(&IsolateState) -> R) -> R {
        let state = scope.get_slot::<Self>().unwrap();
        f(state)
    }

    /// Run function `f` on the underlying IsolateState (mutable) from scope
    #[inline(always)]
    pub fn with_mut<'s, R>(scope: &mut v8::PinScope<'s, '_, ()>, f: impl FnOnce(&mut IsolateState) -> R) -> R {
        let state = scope.get_slot_mut::<Self>().unwrap();
        f(state)
    }

    /// Attach a promise tracker for a module
    /// 
    /// Returns the module id
    #[inline(always)]
    pub fn create_module_promise_tracker(&mut self) -> (u64, oneshot::Receiver<V8Result>) {
        let nmptid = self.next_module_promise_tracker_id;
        self.next_module_promise_tracker_id += 1;
        let (tx, rx) = oneshot::channel();
        self.module_promise_tracker.insert(nmptid, tx);
        (nmptid, rx)
    }  

    /// Removes a promise tracker for a module given module id from `create_module_promise_tracker`
    pub fn remove_module_promise_tracker(&mut self, module_id: u64) -> Option<oneshot::Sender<V8Result>> {
        self.module_promise_tracker.remove(&module_id)
    }

    /// Returns the number of pending loop refs (if loop refs > 0, the event loop will continue to wait for events and not break)
    pub fn pending_loop(&self) -> usize {
        let mut base = self.pending_ops.len() + self.module_promise_tracker.len();
        if self.worker_to_embedder_cb.is_some() && self.embedder_to_worker_cb.is_some() {
            base += 1; // the worker to embedder pipe thats defined by the embedder creates a ref that keeps the event loop going
        }
        base
    }

    /// Returns the number of 'work units' we have in motion
    pub fn pending_work_units(&self) -> usize {
        self.pending_ops.len() + self.queue_stream.len()
    }

    /// Returns the number of pending promises in the registry
    pub fn pending_promises_in_registry(&self) -> usize {
        self.pending_ops.len()
    }

    /// Returns the number of pending module promises
    pub fn pending_module_promises(&self) -> usize {
        self.module_promise_tracker.len()
    }

    /// Returns a reference to the underlying queue stream
    pub fn queue_stream(&self) -> &QueueStream {
        &self.queue_stream
    }

    /// Returns a mutable reference to the underlying queue stream
    pub fn queue_stream_mut(&mut self) -> &mut QueueStream {
        &mut self.queue_stream
    }

    /// Returns a reference to the module registry
    /// 
    /// Note: not very usable currently as most fields are private
    pub fn modules(&self) -> &ModuleRegistry {
        &self.modules
    }

    /// Returns a mutable reference to the module registry
    /// 
    /// Note: not very usable currently as most fields are private
    pub fn modules_mut(&mut self) -> &mut ModuleRegistry {
        &mut self.modules
    }

    /// Returns elapsed time in milliseconds
    /// 
    /// The returned time is limited to 100 microsecond resolution for security
    pub fn elapsed_ms(&self) -> f64 {
        let resolution_ms = 0.1; // 100 microseconds
        let raw_elapsed = self.start_time.elapsed().as_secs_f64() * 1000.0;
        let elapsed_fuzzy = (raw_elapsed / resolution_ms).floor() * resolution_ms;

        // Add some jitter to protect against timing attacks
        let jitter = rand::random_range(-0.005..0.005); 
        let mut elapsed_final = elapsed_fuzzy + jitter;

        // Ensure monotonicity
        let last = self.last_reported_time.get();
        if elapsed_final <= last {
            // If jitter pushed us back, return slightly more than last time
            elapsed_final = last + 0.0001; 
        }
        self.last_reported_time.set(elapsed_final);

        elapsed_final
    }
}  
