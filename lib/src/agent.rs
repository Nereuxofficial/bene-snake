use std::sync::Arc;
use std::time::Duration;

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{Move, SnakeId},
};

use crate::mcts::{Node, SEARCH_WORKERS, search_for};

/// Trait that defines a snake agent's decision-making interface.
pub trait Agent: Send + Sync {
    /// Returns the name of this agent for display purposes.
    fn name(&self) -> &str;

    /// Choose a move given the current board state and the snake ID to play as.
    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move;

    /// Optional: Reset any internal state between games.
    fn reset(&mut self) {}
}

impl Agent for Box<dyn Agent> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        (**self).choose_move(board, you)
    }

    fn reset(&mut self) {
        (**self).reset()
    }
}

/// The MCTS-based agent that uses Monte Carlo Tree Search.
pub struct MctsAgent {
    name: String,
    think_time: Duration,
}

impl MctsAgent {
    pub fn new(think_time: Duration) -> Self {
        Self {
            name: "MCTS".to_string(),
            think_time,
        }
    }

    pub fn with_name(name: impl Into<String>, think_time: Duration) -> Self {
        Self {
            name: name.into(),
            think_time,
        }
    }
}

impl Default for MctsAgent {
    fn default() -> Self {
        Self::new(Duration::from_millis(100))
    }
}

impl Agent for MctsAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let root_node = Arc::new(Node::new_root(*board));
        search_for(&root_node, &you, self.think_time, SEARCH_WORKERS);

        if let Some(mv) = root_node.best_move(you) {
            return mv;
        }

        Move::Up
    }
}
