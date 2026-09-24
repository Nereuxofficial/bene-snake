#![feature(iter_collect_into)]

pub mod agent;
mod eval;
pub mod mcts;

pub use agent::{Agent, MctsAgent};
