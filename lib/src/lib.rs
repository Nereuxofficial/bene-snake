pub mod agent;
pub mod escape;
pub mod eval;
pub mod mcts;
pub mod tactical;

pub use agent::{Agent, MctsAgent};
#[cfg(feature = "tracy")]
tracy_client::register_demangler!();
