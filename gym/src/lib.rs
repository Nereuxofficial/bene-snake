//! Snake Gym - A benchmarking framework for Battlesnake AI agents

pub mod agents;
pub mod runner;
pub mod stats;

pub use agents::{HeuristicAgent, HeuristicPolicy, MinimaxAgent, MinimaxPolicy, RandomAgent};
pub use lib::{Agent, MctsAgent};
pub use runner::{GameConfig, run_game, run_tournament, run_tournament_parallel};
pub use stats::{AgentStats, GameResult, HeadToHeadStats, TournamentStats};
