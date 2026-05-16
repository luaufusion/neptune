pub mod stream;
pub mod time;
pub mod web;
pub mod encoding; // Web encoding API
pub mod test;

use crate::{extension::Globals, native::{encoding::EncodingApiGlobals, stream::EmbedderPipeGlobals, test::TestGlobals, time::{PerformanceGlobals, TimerGlobals}, web::StructuredCloneGlobals}};

/// Trait to register all globals
pub trait RegisterAll {
    fn register_globals_<T: Globals>(&mut self);

    /// Register all globals to the runtime
    fn register_all(&mut self) {
        self.register_globals_::<TimerGlobals>();
        self.register_globals_::<PerformanceGlobals>();
        self.register_globals_::<StructuredCloneGlobals>();
        self.register_globals_::<EmbedderPipeGlobals>();
        self.register_globals_::<EncodingApiGlobals>();
        self.register_globals_::<TestGlobals>();
    }
}