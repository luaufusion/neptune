pub mod runtime;
mod buffer;
pub mod cppgc;
pub mod extension;
pub mod stream;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
