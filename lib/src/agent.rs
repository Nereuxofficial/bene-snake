use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{Move, SnakeId},
};

use crate::mcts::{mcts_search, Node};

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
    exploration_constant: f32,
}

impl MctsAgent {
    pub fn new(think_time: Duration) -> Self {
        Self {
            name: "MCTS".to_string(),
            think_time,
            exploration_constant: 0.0,
        }
    }

    pub fn with_name(name: impl Into<String>, think_time: Duration) -> Self {
        Self {
            name: name.into(),
            think_time,
            exploration_constant: 0.0,
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
        let stop = Arc::new(AtomicBool::new(false));

        let stop_clone = Arc::clone(&stop);
        let root_clone = Arc::clone(&root_node);

        let search_thread = std::thread::spawn(move || {
            mcts_search(root_clone, &you, stop_clone);
        });

        std::thread::sleep(self.think_time);
        stop.store(true, Ordering::Relaxed);
        let _ = search_thread.join();

        if let Some((action, _)) = root_node.best_child(self.exploration_constant) {
            let moves = action.into_inner();
            if let Some(mv) = moves[you.0 as usize] {
                return mv;
            }
        }

        Move::Up
    }
}
