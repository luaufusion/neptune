use std::time::Duration;

use neptune::{fsw::FilesystemWrapper, native::{RegisterAll, console::Console}, runtime::PipedMessage, runtime_snapshotter::NeptuneSnapshot, timer::ItemHandler};
use rust_embed::Embed;
use tokio::{runtime::LocalOptions, sync::mpsc};
use v8::CreateParams;

#[derive(Embed, Debug)]
#[folder = "$CARGO_MANIFEST_DIR/test/a"]
#[prefix = ""] 
pub struct Test;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build_local(LocalOptions::default()).expect("Failed to spawn tokio rt");

    let vfs = vfs::EmbeddedFS::<Test>::new();
    let snapshot = NeptuneSnapshot::load("snapshot.bin", |mut ns| {
        ns.register_all();
        ns
    }).expect("Failed to get snapshot");

    rt.block_on(async move {        
        let mut rt = neptune::runtime::JsRuntime::new(
            CreateParams::default(),
            snapshot,
            FilesystemWrapper::new(vfs)
        );
        rt.init_class::<Console>(true);

        println!("Created runtime!");

        // Push a RustCall in
        rt.queue_stream().add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
            println!("[Rust] RustCall on item {item:?}")
        }) }, Duration::from_secs(2));
        rt.queue_stream().add(ItemHandler::RustCall { cb: Box::new(|_scope, item| {
            println!("[Rust] RustCall v2 on item {item:?}");
        }) }, Duration::from_secs(5));

        // Init a pipe for test
        let (tx, mut rx) = mpsc::unbounded_channel::<PipedMessage>();
        rt.set_worker_to_embedder_cb(Some(Box::new(move |msg| {
            let _ = tx.send(msg);
        })));

        if let Err(e) = rt.execute("let _c = new Console(); globalThis.console = _c; console.log(console)") {
            eprintln!("{e}");
        }

        if let Err(e) = rt.execute("console.log('sss' + JSON.stringify({a: 1}));") {
            eprintln!("{e}");
        }

        let handle = match rt.execute_main_module("main.js") {
            Ok(handle) => handle,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };

        tokio::pin!(handle);
        
        loop {
            tokio::select! {
                r = rt.run_event_loop() => {
                    if let Err(e) = r {
                        eprintln!("{e}");
                        return;
                    }
                }
                msg = &mut handle => {
                    if let Err(e) = rt.parse_module_resp(msg.unwrap()) {
                        eprintln!("{e}");
                    }
                    return;
                }
                Some(msg) = rx.recv() => {
                    println!("Worker posted {msg:?}");
                }
                d = tokio::time::sleep(Duration::from_secs(2)) => {
                    if let Err(e) = rt.push_message(PipedMessage::PostedString(format!("Rust says hello to this beautiful worker {d:?}"))) {
                        eprintln!("{e}")
                    }
                }
            }
        }
    });
}