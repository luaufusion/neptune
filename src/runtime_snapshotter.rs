use std::{borrow::Cow, path::Path};

use v8::{ContextOptions, CreateParams};

use crate::{extension::Globals, js::{BootstrapGlobals, load_js}, native::RegisterAll, runtime::V8_INIT};

#[derive(wincode::SchemaWrite, wincode::SchemaRead, serde::Serialize, serde::Deserialize)]
/// Internal snapshot storage format
/// 
/// TODO: Use this to validate snapshot data
pub struct NeptuneSnapshotData {
    magic: [u8; 4],
    globals: Vec<String>,
    flags: Option<String>,
    startup_data: Vec<u8>
}

impl NeptuneSnapshotData {
    const MAGIC:[u8; 4] = [10, 20, 11, 9]; // random 4 bytes
    pub fn from_neptune_snapshot(ns: &NeptuneSnapshot) -> Self {
        let globals = ns.globals.iter().map(|(x, _)| x.to_string()).collect::<Vec<_>>();
        Self { magic: Self::MAGIC, globals, flags: ns.flags.clone(), startup_data: ns.startup_data.to_vec() }
    }

    pub fn into_neptune_snapshot(self, finalizer: impl FnOnce(NeptuneSnapshot) -> NeptuneSnapshot) -> NeptuneSnapshot {
        assert!(self.magic == Self::MAGIC, "neptune snapshot data has incorrect magic");
        let mut ns = NeptuneSnapshot::new_from_data(self.startup_data.into(), Vec::new(), self.flags);

        // We need BootstrapGlobals to be first in list
        ns.register_globals::<BootstrapGlobals>();

        let ns = (finalizer)(ns);

        let tgt_globals = ns.globals.iter().map(|(x, _)| x.to_string()).collect::<Vec<_>>();

        assert!(self.globals == tgt_globals, "neptune snapshots and target snapshot differ");

        ns
    }
}

/// A NeptuneSnapshot that can be used to start a NeptuneRuntime with JsRuntime
/// 
/// The order in which register_globals is called must be the same as when the snapshot is created.
/// Also the flags passed must also be the same
pub struct NeptuneSnapshot {
    pub(super) startup_data: v8::StartupData,
    pub(super) globals: Vec<(&'static str, v8::FunctionCallback)>,
    pub(super) flags: Option<String>,
}

impl NeptuneSnapshot {
    /// Create a new neptune snapshot directly from snapshot_data
    /// 
    /// You probably want to use NeptuneSnapshotData here
    pub unsafe fn new_unchecked(startup_data: Vec<u8>, flags: Option<String>) -> Self {
        let mut s = Self { startup_data: startup_data.into(), globals: Vec::new(), flags };

        // We need BootstrapGlobals to be first in list
        s.register_globals::<BootstrapGlobals>();

        s
    }

    pub fn new_from_data(startup_data: v8::StartupData, globals: Vec<(&'static str, v8::FunctionCallback)>, flags: Option<String>) -> Self {
        Self { startup_data, globals, flags }
    }

    pub fn register_globals<T: Globals>(&mut self) {
        self.globals.extend(T::get_external_references());
    }

    pub fn ext_refs(&self) -> Vec<v8::ExternalReference> {
        let refs: Vec<v8::ExternalReference> = self.globals
            .iter()
            .map(|(_, cb)| v8::ExternalReference { function: (*cb).into() })
            .collect();

        refs
    }

    /// Copies the created Neptune Snapshot to a Vec<u8>
    /// 
    /// You probably want to use NeptuneSnapshotData here
    pub fn to_vec(&self) -> Vec<u8> {
        self.startup_data.to_vec()
    }

    /// Helper method to save a Neptune Snapshot to `path` as a NeptuneSnapshotData
    pub fn save<P>(&self, path: P) -> Result<(), crate::Error>
    where P: AsRef<Path>
    {
        let nsd = NeptuneSnapshotData::from_neptune_snapshot(self);
        let nsd_bytes = wincode::serialize(&nsd)?;
        Ok(std::fs::write(path, &nsd_bytes).map_err(|x| x.to_string())?)
    }

    /// Helper method to load a Neptune Snapshot from `path` as a NeptuneSnapshotData
    pub fn load<P>(path: P, finalizer: impl FnOnce(NeptuneSnapshot) -> NeptuneSnapshot) -> Result<Self, crate::Error>
    where P: AsRef<Path>
    {
        let file_contents = std::fs::read(path).map_err(|x| x.to_string())?;
        Self::load_bytes(file_contents, finalizer)
    }

    /// Helper method to load a Neptune Snapshot from `contents` as a NeptuneSnapshotData
    #[inline(always)]
    pub fn load_bytes(contents: Vec<u8>, finalizer: impl FnOnce(NeptuneSnapshot) -> NeptuneSnapshot) -> Result<Self, crate::Error> {
        let nsd: NeptuneSnapshotData = wincode::deserialize(&contents)?;
        Ok(nsd.into_neptune_snapshot(finalizer))
    }
}

impl RegisterAll for NeptuneSnapshot {
    fn register_globals_<T: Globals>(&mut self) {
        self.register_globals::<T>();
    }
}

pub struct JsRuntimeSnapshotter {
    params: CreateParams,
    flags: Option<String>,
    globals: Vec<(&'static str, v8::FunctionCallback)>,
    global_regs: Vec<Box<dyn FnOnce(&mut v8::PinScope, v8::Local<v8::Object>)>>,
    
    // js files
    files: Vec<(Cow<'static, str>, Cow<'static, str>)>
}

impl JsRuntimeSnapshotter {
    pub fn new(params: CreateParams, flags: Option<String>) -> Self {
        let mut s = Self { params, flags, globals: Vec::new(), global_regs: Vec::new(), files: Vec::new() };
        
        // We need BootstrapGlobals to be first in list
        s.register_globals::<BootstrapGlobals>();

        s
    }

    pub fn register_globals<T: Globals>(&mut self) {
        self.globals.extend(T::get_external_references());
        self.files.extend(T::js_files());
        self.global_regs.push(Box::new(|scope, global| {
            T::register(scope, global);
        }));
    }

    pub fn ext_refs(&self) -> Vec<v8::ExternalReference> {
        let refs: Vec<v8::ExternalReference> = self.globals
            .iter()
            .map(|(_, cb)| v8::ExternalReference { function: (*cb).into() })
            .collect();

        refs
    }

    #[inline(always)]
    pub(super) fn create_global_context(isolate: &mut v8::Isolate) -> v8::Global<v8::Context> {
        v8::scope!(let scope, isolate);
        
        // Create a template for the global object (`window` / `globalThis`)
        let global_template = v8::ObjectTemplate::new(scope);

        // Instantiate the context
        let context = v8::Context::new(scope, ContextOptions {
            global_template: Some(global_template),
            ..Default::default()
        });
        v8::Global::new(scope, context)
    }

    pub fn finalize(self, init_code: Option<&str>) -> NeptuneSnapshot {
        V8_INIT.call_once(|| {
            if let Some(flags) = &self.flags {
                v8::V8::set_flags_from_string(flags);
            }
            let platform = v8::new_default_platform(0, true).make_shared();
            v8::cppgc::initialize_process(platform.clone());
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        let refs: Vec<v8::ExternalReference> = self.ext_refs();

        // Create isolate and set state inside of a slot
        let platform = v8::V8::get_current_platform();
        let cpp_heap = v8::cppgc::Heap::create(
            platform,
            v8::cppgc::HeapCreateParams::default(),
        );

        let isolate = v8::Isolate::snapshot_creator(Some(Cow::Owned(refs)), Some(self.params.cpp_heap(cpp_heap)));

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
            let global_context = Self::create_global_context(isolate.as_mut());

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
            if let Some(code) = init_code {
                let code = v8::String::new(&mut scope, code).unwrap();

                let script = v8::Script::compile(&mut scope, code, None).unwrap();
                script.run(&mut scope).expect("Failed to run prelude during snapshotting");
            }
        }

        NeptuneSnapshot::new_from_data(
            isolate.take().create_blob(v8::FunctionCodeHandling::Clear).expect("Failed to create snapshot"), 
            self.globals,
            self.flags
        )
    }
}

impl RegisterAll for JsRuntimeSnapshotter {
    fn register_globals_<T: Globals>(&mut self) {
        self.register_globals::<T>();
    }
}