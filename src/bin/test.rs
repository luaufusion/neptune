use std::{rc::Rc, time::Duration};

use futures::FutureExt;
use neptune::{fsw::FilesystemWrapper, native::RegisterAll, runtime::{ConsoleLogMode, EventLoopStatus, JsRuntime, LogMessage, PipedMessage}, runtime_snapshotter::NeptuneSnapshot, timer::ItemHandler};
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

        // Init console log cb
        rt.set_embedder_log_cb(Some(Rc::new(|log_msg| {
            match log_msg {
                LogMessage::ConsoleLog { mode, msg } => {
                    match mode {
                        ConsoleLogMode::Error => {
                            #[cfg(feature = "console")]
                            {
                                use colored::*;
                                eprintln!("{}", msg.red())
                            }
                        }
                        ConsoleLogMode::Log => {
                            println!("{msg}")
                        }
                    }
                },
                LogMessage::DbgOnModuleAsyncDone | LogMessage::DbgOnModuleAsyncError => {},
                _ => {
                    let msg = log_msg.repr();
                    #[cfg(feature = "console")]
                    {
                        use colored::*;
                        eprintln!("{}: {msg}", "error".red().bold());
                    }
                }
            }
        })));

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
        let mut handle = handle.fuse();

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

        loop {
            tokio::select! {
                r = rt.tick() => {
                    if r == EventLoopStatus::Idle { 
                        return;
                    } else if r == EventLoopStatus::Ok {
                        continue; // event loop not done yet
                    } else {
                        #[cfg(feature = "console")]
                        {
                            use colored::*;
                            eprintln!("{}: {}", "error".red().bold(), r.repr());
                        }   
                        return;
                    }
                }
                Ok(msg) = &mut handle => {
                    if let Err(e) = msg {
                        let e = rt.with_context(e, |scope, e| {
                            let e = v8::Local::new(scope, e);
                            JsRuntime::local_to_error(scope, e) 
                        });

                        #[cfg(feature = "console")]
                        {
                            use colored::*;
                            eprintln!("{}: {e}", "error".red().bold());
                        }   
                    }
                    //return;
                }
                Some(msg) = rx.recv() => {
                    println!("Worker posted {msg:?}");
                }
                d = interval.tick() => {
                    if let Err(e) = rt.push_message(PipedMessage::PostedString(format!("Rust says hello to this beautiful worker {d:?}"))) {
                        eprintln!("{e}")
                    }
                }
            }
        }
    });
}