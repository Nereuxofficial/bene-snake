use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
        NeighborDeterminableGame, ReasonableMovesGame, SimulableGame, SimulatorInstruments,
        SnakeId,
    },
};

use lib::Agent;

#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}

/// A heuristic-based agent that uses simple rules to make decisions:
/// - Avoid walls and other snakes
/// - Seek food when health is low
/// - Prefer moves that maximize available space
pub struct HeuristicAgent {
    name: String,
    /// Health threshold below which the snake prioritizes food
    hunger_threshold: u8,
}

impl HeuristicAgent {
    pub fn new() -> Self {
        Self {
            name: "Heuristic".to_string(),
            hunger_threshold: 30,
        }
    }

    pub fn with_name(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            hunger_threshold: 30,
        }
    }

    pub fn with_hunger_threshold(mut self, threshold: u8) -> Self {
        self.hunger_threshold = threshold;
        self
    }

    fn score_move(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        mv: Move,
    ) -> i32 {
        let head = board.get_head_as_native_position(&you);
        let health = board.get_health(&you);
        let length = board.get_length(&you);

        // Simulate the move to see the resulting board
        let moves_for_sim: Vec<_> = board
            .reasonable_moves_for_each_snake()
            .map(|(sid, moves)| {
                let chosen = if sid == you {
                    mv
                } else {
                    // Assume other snakes move randomly - just pick first valid move
                    moves.into_iter().next().unwrap_or(Move::Up)
                };
                (sid, [chosen])
            })
            .collect();

        let Some((_, next_board)) = board
            .simulate_with_moves(&Instr, &moves_for_sim)
            .next()
        else {
            return i32::MIN; // Move results in death
        };

        // Check if we're still alive after the move
        if next_board.get_health(&you) == 0 {
            return i32::MIN;
        }

        let mut score: i32 = 0;

        // Reward having more available moves (space control)
        let next_head = next_board.get_head_as_native_position(&you);
        let available_moves = next_board.possible_moves(&next_head).count() as i32;
        score += available_moves * 10;

        // If hungry, prioritize getting closer to food
        if health < self.hunger_threshold {
            let food_positions = board.get_all_food_as_positions();
            if !food_positions.is_empty() {
                // Find closest food
                let head_pos = board.get_head_as_position(&you);
                let mut min_dist = i32::MAX;
                for food_pos in &food_positions {
                    let dist = (head_pos.x as i32 - food_pos.x as i32).abs()
                        + (head_pos.y as i32 - food_pos.y as i32).abs();
                    min_dist = min_dist.min(dist);
                }
                // Bonus for being close to food when hungry
                score += (20 - min_dist).max(0) * 5;
            }
        }

        // Bonus for length (longer is better)
        score += length as i32;

        // Penalty for low health
        if health < 20 {
            score -= (20 - health as i32) * 2;
        }

        score
    }
}

impl Default for HeuristicAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl Agent for HeuristicAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let reasonable_moves: Vec<Move> = board
            .reasonable_moves_for_each_snake()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| moves.into_iter().collect())
            .unwrap_or_else(|| vec![Move::Up, Move::Down, Move::Left, Move::Right]);

        // Score each move and pick the best
        reasonable_moves
            .into_iter()
            .max_by_key(|&mv| self.score_move(board, you, mv))
            .unwrap_or(Move::Up)
    }
}
