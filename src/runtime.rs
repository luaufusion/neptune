use std::{any::TypeId, collections::HashMap};
use std::sync::Once;
use tokio::sync::mpsc;
use v8::{ContextOptions, CreateParams};

use crate::buffer::v8_new_array_buffer;
use crate::extension::NativeObject;
use crate::fsw::FilesystemWrapper;
use crate::module::{ModuleRegistry, create_module_origin, module_resolve_callback};

/// The message passed from Tokio background tasks back to V8
pub(super) struct AsyncResult {
    pub(super) promise_id: usize,
    pub(super) result: Result<Value, crate::Error>,
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

/// Internal v8 isolate state
pub(super) struct IsolateState {
    pub(super) tx: mpsc::UnboundedSender<AsyncResult>,
    pub(super) promise_registry: HashMap<usize, v8::Global<v8::PromiseResolver>>,
    pub(super) next_promise_id: usize,
    pub(super) pending_promises: usize,
    pub(super) on_module_async_done_fn: v8::Global<v8::Function>,


    // needed for cppgc
    pub(super) cppgc_fallback_template: v8::Global<v8::ObjectTemplate>,
    pub(super) cppgc_type_templates: HashMap<TypeId, v8::Global<v8::FunctionTemplate>>,

    // modules
    pub(super) modules: ModuleRegistry
}

// Ensure V8 is only initialized once per process
static V8_INIT: Once = Once::new();

pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
    global_context: v8::Global<v8::Context>,
    rx: mpsc::UnboundedReceiver<AsyncResult>,
}

impl JsRuntime {
    pub fn new(params: CreateParams, flags: Option<String>, vfs: FilesystemWrapper) -> Self {
        // Init v8 platform if needed
        V8_INIT.call_once(|| {
            if let Some(flags) = flags {
                v8::V8::set_flags_from_string(&flags);
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
        let mut isolate = v8::Isolate::new(params.cpp_heap(cpp_heap));

        isolate.set_promise_reject_callback(promise_reject_callback);

        let cppgc_fallback_template = {
            v8::scope!(let scope, &mut isolate);
            let tpl = v8::ObjectTemplate::new(scope);
            // CRITICAL: Must be exactly 2 for cppgc to work
            tpl.set_internal_field_count(2); 
            v8::Global::new(scope, tpl)
        };

        // Create global context
        let global_context = {
            v8::scope!(let scope, &mut isolate);
            
            // Create a template for the global object (`window` / `globalThis`)
            let global_template = v8::ObjectTemplate::new(scope);

            // Instantiate the context
            let context = v8::Context::new(scope, ContextOptions {
                global_template: Some(global_template),
                ..Default::default()
            });
            v8::Global::new(scope, context)
        };

        let on_module_async_done_fn = {
            v8::scope!(let scope, &mut isolate);
            let gc = v8::Local::new(scope, global_context.clone());
            let scope = &mut v8::ContextScope::new(scope, gc);
            let on_settled_fn = v8::Function::new(scope, on_module_async_done).unwrap();
            v8::Global::new(scope, on_settled_fn)
        };

        isolate.set_slot(IsolateState {
            tx,
            promise_registry: HashMap::new(),
            next_promise_id: 1,
            pending_promises: 0,
            on_module_async_done_fn,
            cppgc_fallback_template,
            cppgc_type_templates: HashMap::new(),
            modules: ModuleRegistry::new(vfs)
        });

        Self {
            isolate,
            global_context,
            rx,
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

    pub fn execute_main_module(&mut self, path: &str) -> Result<(), crate::Error> {
        v8::scope!(let scope, &mut self.isolate);
        let global_context = v8::Local::new(scope, &self.global_context);
        let context = v8::Local::new(scope, global_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        v8::tc_scope!(let scope, scope);

        // Fetch the source code for the main entry point
        let source_code = {
            let state = scope.get_slot::<IsolateState>().unwrap();
            match state.modules.vfs.get_file(path.to_string()) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => {
                    return Err(format!("Failed to read main module {}: {:?}", path, e).into());
                }
            }
        };

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

        // Register the main module to ensure referrer is known in module_resolve_callback
        let hash = main_module.get_identity_hash();
        let g_mod = v8::Global::new(scope, main_module);
        {
            let state = scope.get_slot_mut::<IsolateState>().unwrap();
            state.modules.cache.insert(path.to_string(), g_mod);
            state.modules.paths.insert(hash.into(), path.to_string());
        }

        // Instantiate the module graph
        // This recursively triggers your module_resolve_callback for all dependencies!
        if main_module.instantiate_module(scope, module_resolve_callback).is_none() {
            if let Some(exception) = scope.exception() {
                let msg = exception.to_rust_string_lossy(scope);
                return Err(format!("Failed to link module graph: {msg}").into())
            } else {
                return Err("Unknown error has occurred".into())
            }
        }

        // Execute the code
        match main_module.evaluate(scope) {
            Some(v) => {
                // handle promises
                if v.is_promise() {
                    let promise = v8::Local::<v8::Promise>::try_from(v)?;
                        if promise.state() == v8::PromiseState::Pending {
                            // if pending, incr pending so event loop stays alive, then set module promise then/catch handler
                            {
                                let state = scope.get_slot_mut::<IsolateState>().unwrap();
                                state.pending_promises += 1;
                            }

                            let on_module_async_done_fn = {
                                let state = scope.get_slot::<IsolateState>().unwrap();
                                v8::Local::new(scope, state.on_module_async_done_fn.clone())
                            };

                            promise.then(scope, on_module_async_done_fn).unwrap();
                            promise.catch(scope, on_module_async_done_fn).unwrap();
                        }
                }

                // perform microtask checkpoint
                scope.perform_microtask_checkpoint();   

                Ok(()) 
            },
            None => {
                // perform microtask checkpoint
                scope.perform_microtask_checkpoint();    

                if let Some(exception) = scope.exception() {
                    let msg = exception.to_rust_string_lossy(scope);
                    return Err(format!("Error executing main module: {msg}").into())
                } else {
                    return Err("Unknown error has occurred".into())
                }
            }

        }
    }

    pub async fn execute_main_module_async(&mut self, path: &str) -> Result<(), crate::Error> {
        self.execute_main_module(path)?;
        self.run_event_loop().await;
        Ok(())
    }

    /// Runs the Tokio event loop until all Promises are resolved
    pub async fn run_event_loop(&mut self) {
        loop {
            // Check if we have pending promises
            let pending = {
                v8::scope!(let scope, &mut self.isolate);
                let state = scope.get_slot::<IsolateState>().unwrap();
                state.pending_promises
            };

            if pending == 0 {
                break; // Exit loop when all async work is done
            }

            // wait for bg task to complete
            match self.rx.recv().await {
                Some(msg) => {
                    v8::scope!(let scope, &mut self.isolate);
                    let context = v8::Local::new(scope, &self.global_context);
                    let mut context_scope = v8::ContextScope::new(scope, context);

                    let global_resolver_opt = {
                        let state = context_scope.get_slot_mut::<IsolateState>().unwrap();
                        let resolver = state.promise_registry.remove(&msg.promise_id);
                        
                        if resolver.is_some() {
                            state.pending_promises -= 1;
                        }
                        resolver
                    }; // state dropped here

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
                None => break
            }
        }
    }
}

/// When the module is done, on_module_async_done will be called
fn on_module_async_done<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    println!("on_module_async_done");
    let state = scope.get_slot_mut::<IsolateState>().unwrap();
    state.pending_promises -= 1;
}

// v8 will call this for every promise that has been rejected but unhandled
pub unsafe extern "C" fn promise_reject_callback(message: v8::PromiseRejectMessage) {
    let cbs = std::pin::pin!(unsafe { v8::CallbackScope::new(&message) });
    let scope = &mut cbs.init();
    let event = message.get_event();
    
    match event {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            let exception = message.get_value().unwrap();
            let msg = exception.to_rust_string_lossy(scope);
            eprintln!("UnhandledPromiseRejectionWarning: {}", msg);
            //scope.terminate_execution(); [unsure on this]
        }
        _ => {}
    }
}