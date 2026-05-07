use std::{any::TypeId, collections::HashMap};
use std::sync::Once;
use tokio::sync::mpsc;
use v8::{ContextOptions, CreateParams};

use crate::buffer::v8_new_array_buffer;
use crate::extension::NativeObject;

/// The message passed from Tokio background tasks back to V8
pub(super) struct AsyncResult {
    pub(super) promise_id: usize,
    pub(super) result: Result<Value, crate::Error>,
}

pub enum Value {
    Global(v8::Global<v8::Value>),
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

    // needed for cppgc
    pub(super) cppgc_fallback_template: v8::Global<v8::ObjectTemplate>,
    pub(super) cppgc_type_templates: HashMap<TypeId, v8::Global<v8::FunctionTemplate>>,
}

// Ensure V8 is only initialized once per process
static V8_INIT: Once = Once::new();

// ---------------------------------------------------------
// 2. The Runtime Struct
// ---------------------------------------------------------

pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
    global_context: v8::Global<v8::Context>,
    rx: mpsc::UnboundedReceiver<AsyncResult>,
}

impl JsRuntime {
    pub fn new(params: CreateParams, flags: Option<String>) -> Self {
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

        let cppgc_fallback_template = {
            v8::scope!(let scope, &mut isolate);
            let tpl = v8::ObjectTemplate::new(scope);
            // CRITICAL: Must be exactly 2 for cppgc to work
            tpl.set_internal_field_count(2); 
            v8::Global::new(scope, tpl)
        };

        isolate.set_slot(IsolateState {
            tx,
            promise_registry: HashMap::new(),
            next_promise_id: 1,
            pending_promises: 0,
            cppgc_fallback_template,
            cppgc_type_templates: HashMap::new(),
        });

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

        Self {
            isolate,
            global_context,
            rx,
        }
    }

    /// Register a NativeObject with the runtime
    pub fn init_class<T: NativeObject>(&mut self) {
        let global_template = {
            v8::scope!(let scope, &mut self.isolate);
                        
            let template = T::setup(scope);
            v8::Global::new(scope, template)
        };

        v8::scope!(let scope, &mut self.isolate); 
        let state = scope.get_slot_mut::<IsolateState>().unwrap();
        state.cppgc_type_templates.insert(std::any::TypeId::of::<T>(), global_template);
    }

    /// Executes synchronous JavaScript code
    pub fn execute(&mut self, source_code: &str) {
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &self.global_context);
        let mut context_scope = v8::ContextScope::new(scope, context);

        let code = v8::String::new(&mut context_scope, source_code).unwrap();
        
        if let Some(script) = v8::Script::compile(&mut context_scope, code, None) {
            script.run(&mut context_scope);
        }
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
            if let Some(msg) = self.rx.recv().await {
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
        }
    }
}
