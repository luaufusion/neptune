use std::time::Duration;

use neptune::{fsw::FilesystemWrapper, native::{console::Console, stream::Stream, time::Ticker}, timer::ItemHandler};
use rust_embed::Embed;
use tokio::runtime::LocalOptions;
use v8::CreateParams;

#[derive(Embed, Debug)]
#[folder = "$CARGO_MANIFEST_DIR/test/a"]
#[prefix = ""] 
pub struct Test;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build_local(LocalOptions::default()).expect("Failed to spawn tokio rt");

    let vfs = vfs::EmbeddedFS::<Test>::new();

    rt.block_on(async move {
        let mut rt = neptune::runtime::JsRuntime::new(
            CreateParams::default(),
            Some("--jitless".to_string()),
            FilesystemWrapper::new(vfs)
        );
        rt.init_class::<Stream>(false);
        rt.init_class::<Console>(true);
        rt.init_class::<Ticker>(true);

        println!("Created runtime!");

        // Push a RustCall in
        rt.isolate_state().queue_stream.add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
            println!("[Rust] RustCall on item {item:?}")
        }) }, Duration::from_secs(2));
        rt.isolate_state().queue_stream.add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
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