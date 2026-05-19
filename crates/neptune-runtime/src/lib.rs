extern crate self as neptune_runtime; // needed for op to be usable

pub mod native;
pub mod fsw;
pub mod runtime;
pub mod state;
pub mod timer;
pub mod module;
pub mod buffer;
pub mod extension;
pub mod runtime_snapshotter;
pub mod js;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

mod test_macros {
    use neptune_macros::{FromV8, IntoV8};

    #[derive(FromV8, IntoV8)]
    #[allow(dead_code)]
    struct MyTestStruct {
        pub a: i32,
        pub b: String,
        pub c: Vec<u8>,
    }
}