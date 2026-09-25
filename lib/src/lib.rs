#![feature(iter_collect_into)]

pub mod agent;
pub mod eval;
pub mod mcts;

pub use agent::{Agent, MctsAgent};
