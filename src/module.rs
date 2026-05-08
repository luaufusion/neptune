use std::collections::HashMap;
use std::path::{Component, Path};

use crate::fsw::FilesystemWrapper;
use crate::runtime::IsolateState;

pub(super) struct ModuleRegistry {
    pub(super) cache: HashMap<String, v8::Global<v8::Module>>,
    pub(super) paths: HashMap<i32, String>,
    pub(super) vfs: FilesystemWrapper
}

impl ModuleRegistry {
    pub fn new(vfs: FilesystemWrapper) -> Self {
        Self { cache: HashMap::new(), paths: HashMap::new(), vfs }
    }
    
    /// Resolves an import specifier to an absolute VFS path, checking for extensions
    /// given the referrer_path given by referrer_path
    pub fn resolve_import_path(&self, specifier: &str, referrer_path: &str) -> Result<String, String> {
        let normalized_path = if specifier.starts_with('/') {
            // Absolute Path
            specifier.to_string()
        } else if specifier.starts_with('.') {
            // Relative Path
            let base_dir = Path::new(referrer_path).parent().unwrap_or(Path::new("/"));
            let joined = base_dir.join(specifier);
            
            // Normalize path (evaluate `.` and `..`)
            let mut clean = Vec::new();
            for comp in joined.components() {
                match comp {
                    Component::ParentDir => { clean.pop(); }
                    Component::Normal(c) => clean.push(c.to_str().unwrap()),
                    Component::RootDir => clean.push(""),
                    _ => {}
                }
            }
            
            let mut result = clean.join("/");
            if !result.starts_with('/') { 
                result.insert(0, '/'); 
            }
            result
        } else {
            // Bare Specifier (e.g., "lodash", "http")
            // In a simple VFS, we can map this to the root, or handle aliases here.
            format!("/{}", specifier)
        };

        // Try exact match first: e.g., "/foo.js"
        if self.vfs.is_file(normalized_path.clone()).unwrap_or(false) {
            return Ok(normalized_path);
        }

        // Try appending ".js": e.g., "/foo" -> "/foo.js"
        let path_js = format!("{}.js", normalized_path);
        if self.vfs.is_file(path_js.clone()).unwrap_or(false) {
            return Ok(path_js);
        }

        // Try appending "/index.js": e.g., "/foo" -> "/foo/index.js"
        let path_index = format!("{}/index.js", normalized_path);
        if self.vfs.is_file(path_index.clone()).unwrap_or(false) {
            return Ok(path_index);
        }

        // 3. Fallback/Error
        Err(format!("Cannot resolve module '{}' imported from '{}'", specifier, referrer_path))
    }
}

pub(super) fn create_module_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    filename: &str,
    is_module: bool
) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, filename).unwrap();
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0, 0, false, 0, None, false, false,
        is_module, None
    )
}

pub(super) fn module_resolve_callback<'a>(
    context: v8::Local<'a, v8::Context>,
    specifier: v8::Local<'a, v8::String>,
    _import_assertions: v8::Local<'a, v8::FixedArray>,
    referrer: v8::Local<'a, v8::Module>,
) -> Option<v8::Local<'a, v8::Module>> {
    // Establish scope and extract the requested specifier
    let cbs = std::pin::pin!(unsafe { v8::CallbackScope::new(context) });
    let scope = &mut cbs.init();

    let specifier_str = specifier.to_rust_string_lossy(scope);

    // Identify referrer and target path while handling relative paths etc
    let referrer_hash = referrer.get_identity_hash();

    let referrer_path = IsolateState::with(scope, |state| {
        state.modules.paths.get(&referrer_hash.into())
        .expect("Fatal: Referrer module not found in ModuleRegistry path tracking")
        .clone()
    });

    let target_path = IsolateState::with(scope, |state| {
        match state.modules.resolve_import_path(&specifier_str, &referrer_path) {
            Ok(path) => Some(path),
            Err(err_msg) => {
                // FIX: Split into two lines to satisfy the borrow checker
                let msg = v8::String::new(scope, &err_msg).unwrap();
                let err = v8::Exception::error(scope, msg);
                scope.throw_exception(err);
                return None;
            }
        }
    })?;

    {
        let state = scope.get_slot::<IsolateState>()
            .expect("Fatal: SyncRuntimeState not found in isolate slot");


        if let Some(cached) = state.modules.cache.get(&target_path) {
            return Some(v8::Local::new(scope, cached));
        }
    }

    // slow-path
    let source_bytes = {
        let state = scope.get_slot::<IsolateState>()
            .expect("Fatal: SyncRuntimeState not found in isolate slot");

        match state.modules.vfs.get_file(target_path.clone()) {
            Ok(bytes) => bytes,
            Err(e) => {
                // FIX: Split into two lines to satisfy the borrow checker
                let msg = v8::String::new(scope, &format!("Failed to read file '{}': {:?}", target_path, e)).unwrap();
                let err = v8::Exception::error(scope, msg);
                scope.throw_exception(err);
                return None;
            }
        }
    };
    
    // Convert Vec<u8> to a UTF-8 String
    let source_code = String::from_utf8_lossy(&source_bytes).into_owned();
    let code_v8 = v8::String::new(scope, &source_code).unwrap();
    let origin = create_module_origin(scope, &target_path, true);
    let mut source = v8::script_compiler::Source::new(code_v8, Some(&origin));

    let module = match v8::script_compiler::compile_module(scope, &mut source) {
        Some(m) => m,
        None => {
            // If compilation fails (e.g., SyntaxError in the JS file), 
            // V8 automatically queues the exception internally. 
            // We just return None to let V8 bubble it up to the JS runtime.
            return None; 
        }
    };

    let hash = module.get_identity_hash();
    let g_mod = v8::Global::new(scope, module);

    // This is vital! It allows any imports *inside* this new module 
    // to know their referrer path when this callback fires recursively.
    IsolateState::with_mut(scope, |state| {
        state.modules.cache.insert(target_path.clone(), g_mod);
        state.modules.paths.insert(hash.into(), target_path);
    });
    Some(module)
}