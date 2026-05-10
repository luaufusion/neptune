use std::time::Duration;

use neptune::{fsw::FilesystemWrapper, native::{console::Console, stream::Stream, time::{PerformanceGlobals, TimerGlobals}, web::StructuredCloneGlobals}, timer::ItemHandler};
use rust_embed::Embed;
use tokio::runtime::LocalOptions;
use v8::{ContextOptions, CreateParams};

#[derive(Embed, Debug)]
#[folder = "$CARGO_MANIFEST_DIR/test/a"]
#[prefix = ""] 
pub struct Test;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build_local(LocalOptions::default()).expect("Failed to spawn tokio rt");

    let vfs = vfs::EmbeddedFS::<Test>::new();

    rt.block_on(async move {
        {
            // Snapshotting test
            let mut snap_rt = neptune::runtime_snapshotter::JsRuntimeSnapshotter::new(
                CreateParams::default(),
                None
            );
            snap_rt.register_globals::<TimerGlobals>();
            snap_rt.register_globals::<PerformanceGlobals>();
            snap_rt.register_globals::<StructuredCloneGlobals>();

            let ext_refs = snap_rt.ext_refs(); 
            let blob = snap_rt.finalize(Some(r#"
globalThis.SUCCESS = 132

class Ticker {
    constructor(n) {
        this.n = n
        this.timerId = null;
    }

    async tick() {
        return new Promise((resolve) => {
            this.timerId = setTimeout(resolve, this.n, 123);
        });
    }

    stop() {
        if (this.timerId) {
            clearTimeout(this.timerId);
            this.timerId = null;
        }
    }
}
"#));

            // Load it back
            let params = v8::Isolate::create_params()
            .snapshot_blob(blob)
            .external_references(ext_refs.into());
            let mut isolate = v8::Isolate::new(params);
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, ContextOptions::default());
            let mut scope = v8::ContextScope::new(scope, context);

            // Check if the baked variable exists
            let code = v8::String::new(&mut scope, "SUCCESS").unwrap();
            let script = v8::Script::compile(&mut scope, code, None).unwrap();
            let result = script.run(&mut scope).unwrap();

            assert_eq!(result.to_rust_string_lossy(&mut scope), "132");
            println!("Snapshot test verified!");
        }

        let mut rt = neptune::runtime::JsRuntime::new(
            CreateParams::default(),
            None, //Some("--jitless".to_string()),
            FilesystemWrapper::new(vfs)
        );
        rt.init_class::<Stream>(false);
        rt.init_class::<Console>(true);
        rt.register_globals::<TimerGlobals>();
        rt.register_globals::<PerformanceGlobals>();
        rt.register_globals::<StructuredCloneGlobals>();

        println!("Created runtime!");

        // Push a RustCall in
        rt.queue_stream().add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
            println!("[Rust] RustCall on item {item:?}")
        }) }, Duration::from_secs(2));
        rt.queue_stream().add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
            println!("[Rust] RustCall v2 on item {item:?}")
        }) }, Duration::from_secs(5));


        if let Err(e) = rt.execute("let _c = new Console(); globalThis.console = _c; console.log(console)") {
            eprintln!("{e}");
        }

        if let Err(e) = rt.execute("console.log('sss' + JSON.stringify({a: 1}));") {
            eprintln!("{e}");
        }

        if let Err(e) = rt.execute_main_module_async("main.js").await {
            eprintln!("{e}");
        }
    });
}