use std::borrow::Cow;

use v8::CreateParams;

use crate::{extension::SnapshottableGlobals, runtime::{JsRuntime, V8_INIT}};

pub struct JsRuntimeSnapshotter {
    params: CreateParams,
    flags: Option<String>,
    globals: Vec<v8::FunctionCallback>,
    global_regs: Vec<Box<dyn FnOnce(&mut v8::PinScope, v8::Local<v8::Object>)>>,
}

impl JsRuntimeSnapshotter {
    pub fn new(params: CreateParams, flags: Option<String>) -> Self {
        Self { params, flags, globals: Vec::new(), global_regs: Vec::new() }
    }

    pub fn register_globals<T: SnapshottableGlobals>(&mut self) {
        self.globals.extend(T::get_external_references());
        self.global_regs.push(Box::new(|scope, global| {
            T::register(scope, global);
        }));
    }

    pub fn ext_refs(&self) -> Vec<v8::ExternalReference> {
        let refs: Vec<v8::ExternalReference> = self.globals
            .iter()
            .map(|cb| v8::ExternalReference { function: (*cb).into() })
            .collect();

        refs
    }

    pub fn finalize(self, mode: v8::FunctionCodeHandling, code: Option<&str>) -> v8::StartupData {
        V8_INIT.call_once(|| {
            if let Some(flags) = self.flags {
                v8::V8::set_flags_from_string(&flags);
            }
            let platform = v8::new_default_platform(0, true).make_shared();
            v8::cppgc::initialize_process(platform.clone());
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        let refs: Vec<v8::ExternalReference> = self.globals
            .into_iter()
            .map(|cb| v8::ExternalReference { function: cb.into() })
            .collect();

        let mut isolate = v8::Isolate::snapshot_creator(Some(Cow::Owned(refs)), Some(self.params));

        // Create global context and register anything we need to register now
        {
            let global_context = JsRuntime::create_global_context(&mut isolate);

            v8::scope!(let scope, &mut isolate); 
            let context = v8::Local::new(scope, &global_context);
            scope.set_default_context(context);

            let mut scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);

            for reg in self.global_regs {
                reg(&mut scope, global);
            }

            if let Some(code) = code {
                let code = v8::String::new(&mut scope, code).unwrap();

                let script = v8::Script::compile(&mut scope, code, None).unwrap();
                script.run(&mut scope).expect("Failed to run prelude during snapshotting");
            }
        }

        isolate.create_blob(mode).expect("Failed to create snapshot")
    }
}