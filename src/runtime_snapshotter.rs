use std::borrow::Cow;

use v8::CreateParams;

use crate::{extension::Globals, js::load_js, runtime::{JsRuntime, V8_INIT}};

pub struct JsRuntimeSnapshotter {
    params: CreateParams,
    flags: Option<String>,
    globals: Vec<v8::FunctionCallback>,
    global_regs: Vec<Box<dyn FnOnce(&mut v8::PinScope, v8::Local<v8::Object>)>>,
    
    // js files
    files: Vec<(Cow<'static, str>, Cow<'static, str>)>
}

impl JsRuntimeSnapshotter {
    pub fn new(params: CreateParams, flags: Option<String>) -> Self {
        Self { params, flags, globals: Vec::new(), global_regs: Vec::new(), files: Vec::new() }
    }

    pub fn register_globals<T: Globals>(&mut self) {
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

    pub fn finalize(self, code: Option<&str>) -> v8::StartupData {
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

        let isolate = v8::Isolate::snapshot_creator(Some(Cow::Owned(refs)), Some(self.params));

        // Add drop guard to ensure a panic always calls create_blob
        struct IsoWrapper {
            isolate: Option<v8::OwnedIsolate>,
        }

        impl IsoWrapper {
            fn as_mut(&mut self) -> &mut v8::OwnedIsolate { self.isolate.as_mut().expect("isolate already dropped") }
            fn take(&mut self) -> v8::OwnedIsolate { self.isolate.take().expect("isolate already dropped") }
        }

        impl Drop for IsoWrapper {
            fn drop(&mut self) {
                if let Some(iso) = self.isolate.take() { iso.create_blob(v8::FunctionCodeHandling::Clear); }
            }
        }
        
        let mut isolate = IsoWrapper { isolate: Some(isolate) };

        // Create global context and register anything we need to register now
        {
            let global_context = JsRuntime::create_global_context(isolate.as_mut());

            v8::scope!(let scope, isolate.as_mut()); 
            let context = v8::Local::new(scope, &global_context);
            scope.set_default_context(context);

            let mut scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);

            for reg in self.global_regs {
                reg(&mut scope, global);
            }

            // Load in our JS code
            load_js(scope, self.files).expect("Failed to load js");

            // Load extra init code *after* everything else
            if let Some(code) = code {
                let code = v8::String::new(&mut scope, code).unwrap();

                let script = v8::Script::compile(&mut scope, code, None).unwrap();
                script.run(&mut scope).expect("Failed to run prelude during snapshotting");
            }
        }

        isolate.take().create_blob(v8::FunctionCodeHandling::Clear).expect("Failed to create snapshot")
    }
}