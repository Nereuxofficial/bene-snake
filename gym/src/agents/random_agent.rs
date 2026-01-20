use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{Move, RandomReasonableMovesGame, ReasonableMovesGame, SnakeId},
};
use rand::Rng;

use lib::Agent;

/// A simple agent that picks a random valid move each turn.
/// Useful as a baseline for benchmarking.
pub struct RandomAgent {
    name: String,
}

impl RandomAgent {
    pub fn new() -> Self {
        Self {
            name: "Random".to_string(),
        }
    }

    pub fn with_name(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Default for RandomAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl Agent for RandomAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let mut rng = rand::rng();

        // Try to get a random reasonable move
        if let Some((_, mv)) = board
            .random_reasonable_move_for_each_snake(&mut rng)
            .find(|(sid, _)| *sid == you)
        {
            return mv;
        }

        // Fallback: pick any reasonable move
        if let Some(mv) = board
            .reasonable_moves_for_each_snake()
            .find(|(sid, _)| *sid == you)
            .and_then(|(_, moves)| moves.into_iter().next())
        {
            return mv;
        }

        // Last resort: random direction (snake is probably dead anyway)
        let moves = [Move::Up, Move::Down, Move::Left, Move::Right];
        moves[rng.random_range(0..4)]
    }
}
