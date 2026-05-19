use neptune::native::RegisterAll;
use tokio::runtime::LocalOptions;
use v8::CreateParams;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build_local(LocalOptions::default()).expect("Failed to spawn tokio rt");

    rt.block_on(async move {
        let snapshot = {
            let mut snap_rt = neptune::runtime_snapshotter::JsRuntimeSnapshotter::new(
                CreateParams::default(),
                Some("--jitless".to_string()),
            );
            snap_rt.register_all();

            snap_rt.finalize(None)
        };
        
        snapshot.save("snapshot.bin").expect("Failed to setup snapshot")
    });
}