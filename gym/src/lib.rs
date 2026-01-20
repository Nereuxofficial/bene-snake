//! Snake Gym - A benchmarking framework for Battlesnake AI agents

pub mod agents;
pub mod runner;
pub mod stats;

pub use lib::{Agent, MctsAgent};
pub use agents::{HeuristicAgent, RandomAgent};
pub use runner::{run_game, run_tournament, run_tournament_parallel, GameConfig};
pub use stats::{AgentStats, GameResult, HeadToHeadStats, TournamentStats};
