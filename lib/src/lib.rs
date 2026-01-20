#![feature(float_algebraic)]
#![feature(iter_collect_into)]

pub mod agent;
mod eval;
pub mod mcts;
mod non_pushable_queue;

pub use agent::{Agent, MctsAgent};
