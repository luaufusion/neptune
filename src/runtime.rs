use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Once;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use v8::{ContextOptions, CreateParams};

use crate::buffer::v8_new_array_buffer;
use crate::extension::NativeObject;
use crate::fsw::FilesystemWrapper;
use crate::module::{create_module_origin, module_resolve_callback};
use crate::runtime_snapshotter::NeptuneSnapshot;
use crate::state::{AsyncResult, IsolateState};
use crate::timer::QueueStream;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
/// A message piped between the embedder and worker
pub enum PipedMessage {
    /// Isolate posted bytes
    PostedBytes(Vec<u8>),
    /// Isolate posted a string
    PostedString(String),
}

#[derive(Debug, PartialEq)]
/// A log message piped from the runtime/worker to the embedder
pub enum LogMessage {
    ConsoleLog { msg: String },
    DbgOnModuleAsyncDone,
    DbgOnModuleAsyncError,
    UncaughtPromise { error: String }
}

impl LogMessage {
    pub fn repr(&self) -> Cow<'_, str> {
        match self {
            Self::ConsoleLog { msg } => msg.into(),
            Self::DbgOnModuleAsyncDone => "on_module_async_done".into(),
            Self::DbgOnModuleAsyncError => "on_module_async_error".into(),
            Self::UncaughtPromise {error} => format!("Uncaught (in promise) {error}\n").into()
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum EventLoopStatus {
    Ok,
    Idle,
    TopLevelAwaitPromiseNeverResolved,
    EventLoopUnexpectedlyClosed,
}

impl EventLoopStatus {
    pub fn repr(&self) -> Cow<'_, str> {
        match self {
            Self::Ok => "Ok".into(),
            Self::Idle => "Idle".into(),
            Self::TopLevelAwaitPromiseNeverResolved => "Top-level await promise never resolved".into(),
            Self::EventLoopUnexpectedlyClosed => "The event loop has unexpectedly closed".into()
        }
    }
}

pub enum Value {
    Global(v8::Global<v8::Value>),
    BigIntU64(u64),
    F64(f64),
    String(String),
    Buffer(Vec<u8>),
    Undefined,
    Null
}

impl Value {
    pub fn to_v8<'s>(self, scope: &mut v8::PinScope<'s, '_, ()>) -> v8::Local<'s, v8::Value> {
        match self {
            Value::Global(gv) => {
                let v8_result = v8::Local::new(scope, gv);
                v8_result.into()
            }
            Value::BigIntU64(u) => {
                let v8_result = v8::BigInt::new_from_u64(scope, u);
                v8_result.into()
            }
            Value::F64(u) => {
                let v8_result = v8::Number::new(scope, u);
                v8_result.into()
            }
            Value::String(s) => {
                let v8_result = v8::String::new(scope, &s).unwrap();
                v8_result.into()
            }
            Value::Buffer(buf) => {
                let v8_result = v8_new_array_buffer(scope, &buf, buf.len());
                v8_result.into()
            }
            Value::Undefined => {
                let v8_result = v8::undefined(scope);
                v8_result.into()
            },
            Value::Null => {
                let v8_result = v8::null(scope);
                v8_result.into()
            }
        }
    }
}

type V8Result = Result<(), v8::Global<v8::Value>>;

// Ensure V8 is only initialized once per process
pub(super) static V8_INIT: Once = Once::new();

/// A JsRuntime
pub struct JsRuntime {
    pub(super) isolate: v8::OwnedIsolate,
    pub(super) global_context: v8::Global<v8::Context>,
    pub(super) rx: mpsc::UnboundedReceiver<AsyncResult>,
}

impl JsRuntime {
    pub fn new(params: CreateParams, snapshot: NeptuneSnapshot, vfs: FilesystemWrapper) -> Self {
        assert!(!snapshot.globals.is_empty(), "Attempted to load a NeptuneSnapshot with no external references!");

        // Init v8 platform if needed
        V8_INIT.call_once(|| {
            if let Some(flags) = &snapshot.flags {
                v8::V8::set_flags_from_string(flags);
            }
            let platform = v8::new_default_platform(0, true).make_shared();
            v8::cppgc::initialize_process(platform.clone());
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        // Create async comm channel for event loop handling
        let (tx, rx) = mpsc::unbounded_channel::<AsyncResult>();

        // Create isolate and set state inside of a slot
        let platform = v8::V8::get_current_platform();
        let cpp_heap = v8::cppgc::Heap::create(
            platform,
            v8::cppgc::HeapCreateParams::default(),
        );

        let params = params
            .external_references(Cow::Owned(snapshot.ext_refs()))
            .snapshot_blob(snapshot.startup_data)
            .cpp_heap(cpp_heap);

        let mut isolate = v8::Isolate::new(params);

        isolate.set_promise_reject_callback(promise_reject_callback);
        IsolateState::attach(&mut isolate, tx, vfs);

        // Create global context
        let global_context = {
            v8::scope!(let scope, &mut isolate);
            let global_context = v8::Context::new(scope, ContextOptions::default());
            v8::Global::new(scope, global_context)
        };
        
        Self {
            isolate,
            global_context,
            rx,
        }
    }

    pub fn isolate(&mut self) -> &mut v8::Isolate {
        &mut self.isolate
    }

    pub fn with_context<A, R>(&mut self, a: A, f: impl FnOnce(&mut v8::PinScope, A) -> R) -> R {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.global_context);
        let scope = &mut v8::ContextScope::new(scope, context); 
        f(scope, a)
    }

    pub fn queue_stream(&mut self) -> &mut QueueStream {
        self.isolate.get_slot_mut::<IsolateState>().unwrap().queue_stream_mut()
    }

    /// Set the callback to call when the worker posts a message for the embedder to see
    pub fn set_worker_to_embedder_cb(&mut self, cb: Option<Box<dyn FnMut(PipedMessage)>>) {
        let iso_state = self.isolate.get_slot_mut::<IsolateState>().unwrap();
        iso_state.worker_to_embedder_cb = cb;
    }

    /// Set the callback to call when the embedder posts a message for the worker to see
    pub fn set_embedder_to_worker_cb(&mut self, cb: Option<v8::Global<v8::Function>>) {
        let iso_state = self.isolate.get_slot_mut::<IsolateState>().unwrap();
        iso_state.embedder_to_worker_cb = cb;
    }

    /// Set the callback to call when anything in the runtime wants to push a log event to the embedder
    pub fn set_embedder_log_cb(&mut self, cb: Option<Rc<dyn Fn(LogMessage)>>) {
        let iso_state = self.isolate.get_slot_mut::<IsolateState>().unwrap();
        iso_state.embedder_log_cb = cb;
    }

    /// Push a message for the worker to see, does nothing if theres no embedder_to_worker callback
    pub fn push_message(&mut self, msg: PipedMessage) -> Result<(), crate::Error> {
        let iso_state = self.isolate.get_slot_mut::<IsolateState>().unwrap();
        if let Some(cb) = iso_state.embedder_to_worker_cb.clone() {
            v8::scope!(let scope, &mut self.isolate);
            let global_context = v8::Local::new(scope, &self.global_context);
            let context = v8::Local::new(scope, global_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            v8::tc_scope!(let scope, scope);
            let cb = v8::Local::new(scope, cb);
            let args = match msg {
                PipedMessage::PostedBytes(buf) => v8_new_array_buffer(scope, &buf, buf.len()).into(),
                PipedMessage::PostedString(s) => v8::String::new(scope, &s).unwrap().into()
            };
            let v = cb.call(scope, v8::undefined(scope).into(), &[args]);
            scope.perform_microtask_checkpoint(); // perform microtask checkpoint
            if v.is_none() {
                if let Some(exception) = scope.exception() {
                    let msg = exception.to_rust_string_lossy(scope);
                    
                    return Err(format!("Failed to compile: {msg}").into())
                } else {
                    return Err("Unknown error has occurred".into())
                } 
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    /// Helper method to push a log event to the registered log callback
    /// 
    /// Mainly used by runtime but could be useful for embedders as well
    pub fn push_log_event(&self, evt: LogMessage) {
        let iso_state = self.isolate.get_slot::<IsolateState>().unwrap();
        if let Some(embedder_log_cb) = &iso_state.embedder_log_cb {
            (embedder_log_cb)(evt)
        }
    }

    /// Register a NativeObject with the runtime
    pub fn init_class<T: NativeObject>(&mut self, expose: bool) {
        let global_template = {
            v8::scope!(let scope, &mut self.isolate);
                        
            let template = T::setup(scope);
            v8::Global::new(scope, template)
        };

        v8::scope!(let scope, &mut self.isolate); 
        let state = scope.get_slot_mut::<IsolateState>().unwrap();
        state.cppgc_type_templates.insert(std::any::TypeId::of::<T>(), global_template.clone());

        if expose {
            let context = v8::Local::new(scope, &self.global_context);
            let scope = &mut v8::ContextScope::new(scope, context);

            let global = context.global(scope);
            
            let name_v8 = v8::String::new(scope, T::class_name()).unwrap();

            // Get constructor func and save to global obj
            let template = v8::Local::new(scope, global_template);
            let constructor_func = template.get_function(scope).unwrap();
            global.set(scope, name_v8.into(), constructor_func.into());
        }
    }

    /// Executes synchronous JavaScript code
    pub fn execute(&mut self, source_code: &str) -> Result<(), crate::Error> {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.global_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        v8::tc_scope!(let scope, scope);

        let code = v8::String::new(scope, source_code).unwrap();

        let script = match v8::Script::compile(scope, code, None) {
            Some(s) => s,
            None => {
                if let Some(exception) = scope.exception() {
                    let msg = exception.to_rust_string_lossy(scope);
                    return Err(format!("Failed to compile: {msg}").into())
                } else {
                    return Err("Unknown error has occurred".into())
                } 
            }
        };
        
        if script.run(scope).is_none() {
            if let Some(exception) = scope.exception() {
                let msg = exception.to_rust_string_lossy(scope);
                return Err(format!("Failed to execute: {msg}").into())
            } else {
                return Err("Unknown error has occurred".into())
            } 
        }
        Ok(())
    }

    /// Executes a module. Note that you must call this with tick()
    /// 
    /// The returned oneshot channel must be polled to yield the ending result
    pub fn execute_main_module(&mut self, path: &str) -> Result<oneshot::Receiver<Result<(), v8::Global<v8::Value>>>, crate::Error> {
        /*let module_rx = {
        }; // scope should be dropped at this point

        // Wait for module exec
        tokio::pin!(module_rx);

        loop {
            tokio::select! {
                res = &mut module_rx => {
                    match res {
                        Ok(Ok(_)) => break, // we're done the initial evaluation step
                        Ok(Err(err)) => {
                            v8::scope!(let scope, &mut self.isolate);
                            let global_context = v8::Local::new(scope, &self.global_context);
                            let context = v8::Local::new(scope, global_context);
                            let scope = &mut v8::ContextScope::new(scope, context);

                            let err = v8::Local::new(scope, err);
                            return Err(Self::local_to_error(scope, err).into());
                        },
                        Err(_) => return Err("Tracker dropped".into()),
                    }
                }
                status = self.tick() => {
                    if status == EventLoopStatus::Idle {
                        return Err(format!("Deadlock in {}", path).into());
                    } else if status != EventLoopStatus::Ok {
                        return Err(status.repr().into())
                    }
                }
            }
        }

        // Wait for event loop to be Idle
        loop {
            let state = self.tick().await;
            if state == EventLoopStatus::Idle { return Ok(()) }
            else if state == EventLoopStatus::Ok { continue }
            return Err(state.repr().into())
        }*/

        v8::scope!(let scope, &mut self.isolate);
        let global_context = v8::Local::new(scope, &self.global_context);
        let context = v8::Local::new(scope, global_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        v8::tc_scope!(let scope, scope);

        // Fetch the source code for the main entry point
        let source_code = IsolateState::with(scope, |state| {
            match state.modules().vfs.get_file(path.to_string()) {
                Ok(bytes) => return Ok(String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) => {
                    return Err(format!("Failed to read main module {}: {:?}", path, e));
                }
            }
        })?;

        let code_v8 = v8::String::new(scope, &source_code).unwrap();
        let origin = create_module_origin(scope, &path, true); 
        let mut source = v8::script_compiler::Source::new(code_v8, Some(&origin));

        let main_module = {
            match v8::script_compiler::compile_module(scope, &mut source) {
                Some(m) => m,
                None => {
                    if let Some(exception) = scope.exception() {
                        let msg = exception.to_rust_string_lossy(scope);
                        return Err(format!("Failed to compile: {msg}").into())
                    } else {
                        return Err("Unknown error has occurred".into())
                    }
                }
            }
        };

        // Register the main module to ensure referrer is known when we instantiate the main module and start resolving dependencies
        let hash = main_module.get_identity_hash();
        let g_mod = v8::Global::new(scope, main_module);
        IsolateState::with_mut(scope, |state| {
            state.modules_mut().cache.insert(path.to_string(), g_mod);
            state.modules_mut().paths.insert(hash.into(), path.to_string());
        });

        // Instantiate the main module itself
        if main_module.instantiate_module(scope, module_resolve_callback).is_none() {
            if let Some(exception) = scope.exception() {
                let msg = exception.to_rust_string_lossy(scope);
                return Err(format!("Failed to link module graph: {msg}").into())
            } else {
                return Err("Unknown error has occurred".into())
            }
        }

        // Execute the code
        let eval_result = main_module.evaluate(scope);

        match eval_result {
            Some(v) => {
                // perform microtask checkpoint
                scope.perform_microtask_checkpoint();   

                // track any returned promises
                if v.is_promise() {
                    let promise = v8::Local::<v8::Promise>::try_from(v)?;
                    if promise.state() == v8::PromiseState::Rejected {
                        let error_val = promise.result(scope);
                        let error_msg = error_val.to_rust_string_lossy(scope);
                        return Err(error_msg.into());
                    }
                    Ok(Self::track_module_promise(scope, promise))
                } else {
                    let (tx, rx) = oneshot::channel();
                    let _ = tx.send(Ok(()));
                    Ok(rx)
                }
            },
            None => {
                // perform microtask checkpoint
                scope.perform_microtask_checkpoint();    

                if let Some(exception) = scope.exception() {
                    let msg = Self::local_to_error(scope, exception);
                    return Err(msg.into())
                } else {
                    return Err("Unknown error has occurred".into())
                }
            }
        }
    }

    /// Tracks a module that is async
    pub(crate) fn track_module_promise<'s>(scope: &mut v8::PinnedRef<'_, v8::TryCatch<'s, '_, v8::HandleScope<'_>>>, promise: v8::Local<'s, v8::Promise>) -> oneshot::Receiver<V8Result> {
        if promise.state() == v8::PromiseState::Pending {
            // if pending, attach a tracker so event loop stays alive, then set module promise then/catch handler
            let (id, rx) = IsolateState::with_mut(scope, |state| {
                state.create_module_promise_tracker()
            });

            let on_module_async_done_fn = v8::Function::builder(on_module_async_done)
            .data(v8::BigInt::new_from_u64(scope, id).into())
            .build(scope)
            .unwrap();

            let on_module_async_error_fn = v8::Function::builder(on_module_async_error)
            .data(v8::BigInt::new_from_u64(scope, id).into())
            .build(scope)
            .unwrap();

            promise.then2(scope, on_module_async_done_fn, on_module_async_error_fn).unwrap();

            rx
        } else if promise.state() == v8::PromiseState::Rejected {
            // otherwise, just get out the result
            let result = promise.result(scope);
            let result_g = v8::Global::new(scope, result);
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(Err(result_g));
            rx
        } else {
            // fullfilled
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(Ok(()));
            rx
        }
    }

    /// Runs the Tokio event loop *once*
    /// 
    /// If this returns false, then something went wrong
    pub async fn tick(&mut self) -> EventLoopStatus {
        // Check if we have pending promises
        let (pending, pending_mods, pending_work_units) = {
            let state = self.isolate.get_slot::<IsolateState>().unwrap();
            (state.pending_loop(), state.pending_module_promises(), state.pending_work_units())
        };

        // Heuristic error: if we're currently evaluating a module but have no pending primises
        if pending_mods > 0 && pending_work_units == 0 {
            return EventLoopStatus::TopLevelAwaitPromiseNeverResolved;
        }

        if pending == 0 {
            return EventLoopStatus::Idle; // Return Idle to signal loop is done
        }

        tokio::select! {
            msg = self.rx.recv() => {
                match msg {
                    Some(msg) => {
                        v8::scope!(let scope, &mut self.isolate);
                        let context = v8::Local::new(scope, &self.global_context);
                        let mut context_scope = v8::ContextScope::new(scope, context);

                        let global_resolver_opt = IsolateState::with_mut(&mut context_scope, |state| {
                            state.detach_from_scheduler(msg.promise_id)
                        });

                        if let Some(global_resolver) = global_resolver_opt {
                            match msg.result {
                                Ok(msg) => {
                                    let resolver = v8::Local::new(&mut context_scope, global_resolver);
                                    let v8_result = msg.to_v8(&mut context_scope);
                                    resolver.resolve(&mut context_scope, v8_result.into());
                                }
                                Err(e) => {
                                    let resolver = v8::Local::new(&mut context_scope, global_resolver);
                                    let v8_result = v8::String::new(&mut context_scope, &e.to_string()).unwrap();
                                    resolver.reject(&mut context_scope, v8_result.into());
                                }
                            }

                            // Pump the microtask queue
                            context_scope.perform_microtask_checkpoint();
                        }
                    }
                    None => return EventLoopStatus::EventLoopUnexpectedlyClosed,
                }
            }
            Some(item) = async { 
                self.isolate.get_slot_mut::<IsolateState>().unwrap().queue_stream_mut().next().await 
            } => {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, &self.global_context);
                let mut context_scope = v8::ContextScope::new(scope, context);
                item.handler.handle(&mut context_scope, item.raw);
            }
        }

        return EventLoopStatus::Ok; // mark this tick as a Ok
    }

    pub fn local_to_error<'s>(scope: &mut v8::PinScope<'s, '_>, r: v8::Local<'s, v8::Value>) -> String {
        if let Some(st) = extract_stack_trace(scope, r) {
            st
        } else {
            r.to_rust_string_lossy(scope)
        }
    }
}


/// When the module is done, on_module_async_done will be called
/// 
/// This lets us reap the underlying module's result
fn on_module_async_done<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    IsolateState::with_mut(scope, |state| {
        if let Some(ref embedder_log_cb) = state.embedder_log_cb {
            (embedder_log_cb)(LogMessage::DbgOnModuleAsyncDone);
        }

        if args.data().is_big_int() {
            let n = v8::Local::<v8::BigInt>::try_from(args.data()).unwrap();
            let module_id = n.u64_value().0;

            if let Some(tx) = state.remove_module_promise_tracker(module_id) {
                let _ = tx.send(Ok(()));
            }
        }
    });
}

/// When the module is error'd, on_module_async_error will be called
/// 
/// This lets us reap the underlying module's result
fn on_module_async_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    let error_val = args.get(0);
    let error_val = v8::Global::new(scope, error_val);

    IsolateState::with_mut(scope, |state| {
        if let Some(ref embedder_log_cb) = state.embedder_log_cb {
            (embedder_log_cb)(LogMessage::DbgOnModuleAsyncError);
        }

        if args.data().is_big_int() {
            let n = v8::Local::<v8::BigInt>::try_from(args.data()).unwrap();
            let module_id = n.u64_value().0;

            if let Some(tx) = state.remove_module_promise_tracker(module_id) {
                let _ = tx.send(Err(error_val));
            }
        }
    });
}

// v8 will call this for every promise that has been rejected but unhandled
pub unsafe extern "C" fn promise_reject_callback(message: v8::PromiseRejectMessage) {
    let cbs = std::pin::pin!(unsafe { v8::CallbackScope::new(&message) });
    let scope = &mut cbs.init();
    let log_cb = {
        let iso_state = scope.get_slot::<IsolateState>().unwrap();
        let Some(ref log_cb) = iso_state.embedder_log_cb else {
            return;
        };
        log_cb.clone()
    };

    let event = message.get_event();
    
    match event {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            let exception = message.get_value().unwrap();
            let error = JsRuntime::local_to_error(scope, exception);
            (log_cb)(LogMessage::UncaughtPromise { error });
        }
        _ => {}
    }
}

fn extract_stack_trace<'s>(scope: &mut v8::PinScope<'s, '_>, exception: v8::Local<'s, v8::Value>) -> Option<String> {
    if let Some(obj) = exception.to_object(scope) {
        let stack_key = v8::String::new(scope, "stack").unwrap();
        if let Some(stack) = obj.get(scope, stack_key.into()) {
            if !stack.is_undefined() {
                return Some(format!("{}", stack.to_rust_string_lossy(scope)));
            }
        }
    }

    return None
}