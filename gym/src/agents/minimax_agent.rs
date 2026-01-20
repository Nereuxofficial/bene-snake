use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
        NeighborDeterminableGame, ReasonableMovesGame, SimulableGame, SimulatorInstruments,
        SnakeId, VictorDeterminableGame,
    },
};

use lib::Agent;

#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}

/// A minimax agent with alpha-beta pruning.
pub struct MinimaxAgent {
    name: String,
    depth: u32,
}

impl MinimaxAgent {
    pub fn new(depth: u32) -> Self {
        Self {
            name: "Minimax".to_string(),
            depth,
        }
    }

    pub fn with_name(name: impl Into<String>, depth: u32) -> Self {
        Self {
            name: name.into(),
            depth,
        }
    }

    fn evaluate(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> i32 {
        // Terminal state check
        if board.is_over() {
            return if board.get_winner() == Some(you) {
                10000
            } else {
                -10000
            };
        }

        let health = board.get_health(&you) as i32;
        let length = board.get_length(&you) as i32;
        let head = board.get_head_as_native_position(&you);
        let moves = board.possible_moves(&head).count() as i32;

        // Basic evaluation: health + length * 10 + mobility * 5
        health + length * 10 + moves * 5
    }

    fn minimax(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        depth: u32,
        mut alpha: i32,
        mut beta: i32,
        maximizing: bool,
    ) -> i32 {
        if depth == 0 || board.is_over() {
            return self.evaluate(board, you);
        }

        // Get all possible move combinations
        let snake_moves: Vec<_> = board.reasonable_moves_for_each_snake().collect();

        if snake_moves.is_empty() {
            return self.evaluate(board, you);
        }

        // Generate all move combinations (cartesian product)
        let combinations = Self::generate_move_combinations(&snake_moves);

        if combinations.is_empty() {
            return self.evaluate(board, you);
        }

        if maximizing {
            let mut max_eval = i32::MIN;
            for moves in combinations {
                let moves_for_sim: Vec<_> = moves.iter().map(|(sid, mv)| (*sid, [*mv])).collect();

                if let Some((_, next_board)) = board.simulate_with_moves(&Instr, &moves_for_sim).next() {
                    let eval = self.minimax(&next_board, you, depth - 1, alpha, beta, false);
                    max_eval = max_eval.max(eval);
                    alpha = alpha.max(eval);
                    if beta <= alpha {
                        break;
                    }
                }
            }
            max_eval
        } else {
            let mut min_eval = i32::MAX;
            for moves in combinations {
                let moves_for_sim: Vec<_> = moves.iter().map(|(sid, mv)| (*sid, [*mv])).collect();

                if let Some((_, next_board)) = board.simulate_with_moves(&Instr, &moves_for_sim).next() {
                    let eval = self.minimax(&next_board, you, depth - 1, alpha, beta, true);
                    min_eval = min_eval.min(eval);
                    beta = beta.min(eval);
                    if beta <= alpha {
                        break;
                    }
                }
            }
            min_eval
        }
    }

    fn generate_move_combinations(snake_moves: &[(SnakeId, Vec<Move>)]) -> Vec<Vec<(SnakeId, Move)>> {
        if snake_moves.is_empty() {
            return vec![vec![]];
        }

        let mut result = vec![vec![]];

        for (snake_id, moves) in snake_moves {
            if moves.is_empty() {
                continue;
            }
            let mut new_result = Vec::new();
            for combo in &result {
                for mv in moves {
                    let mut new_combo = combo.clone();
                    new_combo.push((*snake_id, *mv));
                    new_result.push(new_combo);
                }
            }
            result = new_result;
        }

        result
    }
}

impl Default for MinimaxAgent {
    fn default() -> Self {
        Self::new(3)
    }
}

impl Agent for MinimaxAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let my_moves: Vec<Move> = board
            .reasonable_moves_for_each_snake()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| moves.into_iter().collect())
            .unwrap_or_else(|| vec![Move::Up, Move::Down, Move::Left, Move::Right]);

        let mut best_move = my_moves.first().copied().unwrap_or(Move::Up);
        let mut best_score = i32::MIN;

        for mv in my_moves {
            // Create move combination with our move and assume others pick first valid
            let moves_for_sim: Vec<_> = board
                .reasonable_moves_for_each_snake()
                .map(|(sid, moves)| {
                    let chosen = if sid == you {
                        mv
                    } else {
                        moves.into_iter().next().unwrap_or(Move::Up)
                    };
                    (sid, [chosen])
                })
                .collect();

            if let Some((_, next_board)) = board.simulate_with_moves(&Instr, &moves_for_sim).next() {
                let score = self.minimax(&next_board, you, self.depth - 1, i32::MIN, i32::MAX, false);
                if score > best_score {
                    best_score = score;
                    best_move = mv;
                }
            }
        }

        best_move
    }
}
