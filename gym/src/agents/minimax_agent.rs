use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        HeadGettableGame, HealthGettableGame, LengthGettableGame, Move, MoveArray,
        NeighborDeterminableGame, ReasonableMovesGame, SimulableGame, SnakeId,
        VictorDeterminableGame,
    },
};

use lib::Agent;

/// Which search structure a [`MinimaxAgent`] should use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimaxPolicy {
    /// The original search.
    ///
    /// It alternates "maximize" and "minimize" over the cartesian product of
    /// *every* snake's moves, so at a minimizing ply our own future moves are
    /// chosen by the opponent too. It is also terminal checks that use the
    /// perspective-relative `is_over`/`get_winner`. Both make it a poor model of
    /// a simultaneous multiplayer game.
    Legacy,
    /// Paranoid depth-limited search.
    ///
    /// Each ply is one full turn: we choose a move, then the opponents jointly
    /// choose theirs to minimize our value. Our own snake is never adversarially
    /// controlled, and terminal detection is based on who is actually alive.
    Paranoid,
}

/// A minimax agent with alpha-beta pruning.
pub struct MinimaxAgent {
    name: String,
    depth: u32,
    policy: MinimaxPolicy,
}

impl MinimaxAgent {
    pub fn new(depth: u32) -> Self {
        Self {
            name: "Minimax".to_string(),
            depth,
            policy: MinimaxPolicy::Paranoid,
        }
    }

    pub fn with_name(name: impl Into<String>, depth: u32) -> Self {
        Self {
            name: name.into(),
            depth,
            policy: MinimaxPolicy::Paranoid,
        }
    }

    pub fn with_policy(name: impl Into<String>, depth: u32, policy: MinimaxPolicy) -> Self {
        Self {
            name: name.into(),
            depth,
            policy,
        }
    }

    // ------------------------------------------------------------------
    // Paranoid search
    // ------------------------------------------------------------------

    /// Terminal-aware static evaluation for `you`.
    ///
    /// Unlike the legacy evaluation this never uses the perspective-relative
    /// `is_over`/`get_winner`, so it is correct for any snake id and for
    /// multiplayer games.
    fn evaluate(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> i32 {
        if !board.is_alive(&you) {
            return -10000;
        }
        if board.alive_snake_count() == 1 {
            // We are the only snake still alive.
            return 10000;
        }

        let health = board.get_health(&you) as i32;
        let length = board.get_length(&you) as i32;
        let head = board.get_head_as_native_position(&you);
        let moves = board.possible_moves(&head).count() as i32;

        // Basic evaluation: health + length * 10 + mobility * 5
        health + length * 10 + moves * 5
    }

    /// Our move choice. Returns the best value we can guarantee after the
    /// opponents respond to the chosen move.
    fn search(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        depth: u32,
        mut alpha: i32,
        beta: i32,
    ) -> i32 {
        if depth == 0 || !board.is_alive(&you) || board.alive_snake_count() <= 1 {
            return self.evaluate(board, you);
        }

        // Legal moves are the same for every snake at a given board; generate
        // them once and split into our choices and the opponents' choices.
        let snake_moves = board.reasonable_moves_for_each_snake();
        let our_moves: MoveArray = snake_moves
            .iter()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| *moves)
            .unwrap_or_else(|| Move::all().into_iter().collect());
        let opponents: Vec<(SnakeId, MoveArray)> = snake_moves
            .iter()
            .filter(|(sid, _)| *sid != you)
            .map(|(sid, moves)| (*sid, *moves))
            .collect();

        let mut best = i32::MIN;
        for mv in our_moves {
            let value =
                self.minimize_opponents(board, (you, mv), &opponents, depth - 1, alpha, beta);
            if value > best {
                best = value;
            }
            if best > alpha {
                alpha = best;
            }
            if alpha >= beta {
                break;
            }
        }
        best
    }

    /// Value of `our_move` after the opponents jointly pick the response that
    /// minimizes it.
    fn minimize_opponents(
        &self,
        board: &CellBoard4Snakes11x11,
        our_move: (SnakeId, Move),
        opponents: &[(SnakeId, MoveArray)],
        depth: u32,
        alpha: i32,
        mut beta: i32,
    ) -> i32 {
        let (you, our_move) = our_move;
        if opponents.is_empty() {
            let (_, next) = board.simulate_single_action(&[(you, our_move)]);
            return self.search(&next, you, depth, alpha, beta);
        }

        let combinations = opponent_combinations(opponents);
        let mut best = i32::MAX;
        for combination in combinations {
            let mut moves = Vec::with_capacity(combination.len() + 1);
            moves.push((you, our_move));
            moves.extend(combination);
            let (_, next) = board.simulate_single_action(&moves);

            let value = self.search(&next, you, depth, alpha, beta);
            if value < best {
                best = value;
            }
            if best < beta {
                beta = best;
            }
            if alpha >= beta {
                break;
            }
        }
        best
    }

    // ------------------------------------------------------------------
    // Legacy search, retained for deterministic A/B comparison
    // ------------------------------------------------------------------

    fn evaluate_legacy(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> i32 {
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

    fn minimax_legacy(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        depth: u32,
        mut alpha: i32,
        mut beta: i32,
        maximizing: bool,
    ) -> i32 {
        if depth == 0 || board.is_over() {
            return self.evaluate_legacy(board, you);
        }

        // Get all possible move combinations
        let snake_moves = board.reasonable_moves_for_each_snake();

        if snake_moves.is_empty() {
            return self.evaluate_legacy(board, you);
        }

        // Generate all move combinations (cartesian product)
        let combinations = Self::generate_move_combinations(&snake_moves);

        if combinations.is_empty() {
            return self.evaluate_legacy(board, you);
        }

        if maximizing {
            let mut max_eval = i32::MIN;
            for moves in combinations {
                let moves_for_sim: Vec<_> = moves.iter().map(|(sid, mv)| (*sid, [*mv])).collect();

                if let Some((_, next_board)) = board.simulate_with_moves(&moves_for_sim).next() {
                    let eval = self.minimax_legacy(&next_board, you, depth - 1, alpha, beta, false);
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

                if let Some((_, next_board)) = board.simulate_with_moves(&moves_for_sim).next() {
                    let eval = self.minimax_legacy(&next_board, you, depth - 1, alpha, beta, true);
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

    fn generate_move_combinations(
        snake_moves: &[(SnakeId, MoveArray)],
    ) -> Vec<Vec<(SnakeId, Move)>> {
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

    fn choose_move_legacy(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let my_moves: MoveArray = board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| moves)
            .unwrap_or_else(|| Move::all().into_iter().collect());

        let mut best_move = my_moves.first().copied().unwrap_or(Move::Up);
        let mut best_score = i32::MIN;

        for mv in my_moves {
            // Create move combination with our move and assume others pick first valid
            let moves_for_sim: Vec<_> = board
                .reasonable_moves_for_each_snake()
                .into_iter()
                .map(|(sid, moves)| {
                    let chosen = if sid == you {
                        mv
                    } else {
                        moves.into_iter().next().unwrap_or(Move::Up)
                    };
                    (sid, [chosen])
                })
                .collect();

            if let Some((_, next_board)) = board.simulate_with_moves(&moves_for_sim).next() {
                let score = self.minimax_legacy(
                    &next_board,
                    you,
                    self.depth - 1,
                    i32::MIN,
                    i32::MAX,
                    false,
                );
                if score > best_score {
                    best_score = score;
                    best_move = mv;
                }
            }
        }

        best_move
    }
}

/// Cartesian product of every opponent's reasonable moves.
fn opponent_combinations(opponents: &[(SnakeId, MoveArray)]) -> Vec<Vec<(SnakeId, Move)>> {
    let mut result: Vec<Vec<(SnakeId, Move)>> = vec![Vec::new()];
    for (snake_id, moves) in opponents {
        if moves.is_empty() {
            continue;
        }
        let mut next = Vec::with_capacity(result.len() * moves.len());
        for combo in &result {
            for mv in moves {
                let mut extended = combo.clone();
                extended.push((*snake_id, *mv));
                next.push(extended);
            }
        }
        result = next;
    }
    result
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
        match self.policy {
            MinimaxPolicy::Legacy => self.choose_move_legacy(board, you),
            MinimaxPolicy::Paranoid => self.choose_move_paranoid(board, you),
        }
    }
}

impl MinimaxAgent {
    fn choose_move_paranoid(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let snake_moves = board.reasonable_moves_for_each_snake();
        let our_moves: MoveArray = snake_moves
            .iter()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| *moves)
            .unwrap_or_else(|| Move::all().into_iter().collect());
        let opponents: Vec<(SnakeId, MoveArray)> = snake_moves
            .iter()
            .filter(|(sid, _)| *sid != you)
            .map(|(sid, moves)| (*sid, *moves))
            .collect();

        let mut best_move = our_moves.first().copied().unwrap_or(Move::Up);
        let mut best_score = i32::MIN;

        for mv in our_moves {
            let score = self.minimize_opponents(
                board,
                (you, mv),
                &opponents,
                self.depth.saturating_sub(1),
                i32::MIN,
                i32::MAX,
            );
            if score > best_score {
                best_score = score;
                best_move = mv;
            }
        }

        best_move
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{GameConfig, generate_random_game_with_seed};
    use battlesnake_game_types::wire_representation::Position;
    use std::collections::VecDeque;

    type SnakeFixture = (Position, VecDeque<Position>, i32);

    fn fixture(snakes: &[SnakeFixture]) -> CellBoard4Snakes11x11 {
        let config = GameConfig::duel();
        let mut game = generate_random_game_with_seed(&config, 20260931);
        for (snake, (head, body, health)) in game.board.snakes.iter_mut().zip(snakes) {
            snake.head = *head;
            snake.body = body.clone();
            snake.health = *health;
        }
        game.board.food = vec![];
        game.you = game.board.snakes[0].clone();
        let snake_id_map = battlesnake_game_types::types::build_snake_id_map(&game);
        game.as_cell_board(&snake_id_map).unwrap()
    }

    fn open_duel() -> CellBoard4Snakes11x11 {
        let a = Position::new(0, 0);
        let b = Position::new(10, 10);
        fixture(&[
            (a, VecDeque::from([a, a, a]), 100),
            (b, VecDeque::from([b, b, b]), 100),
        ])
    }

    fn is_reasonable(board: &CellBoard4Snakes11x11, you: SnakeId, mv: Move) -> bool {
        board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .find(|(sid, _)| *sid == you)
            .unwrap()
            .1
            .contains(&mv)
    }

    #[test]
    fn default_policy_is_paranoid() {
        assert_eq!(MinimaxAgent::new(2).policy, MinimaxPolicy::Paranoid);
    }

    #[test]
    fn paranoid_evaluation_is_correct_for_nonzero_snake_ids() {
        let board = open_duel();
        let agent = MinimaxAgent::new(1);

        // Two snakes alive: neither is terminal, and the perspective-relative
        // legacy check would have mis-scored the non-zero id.
        assert!(agent.evaluate(&board, SnakeId(0)) > 0);
        assert!(agent.evaluate(&board, SnakeId(1)) > 0);
    }

    #[test]
    fn paranoid_search_returns_a_reasonable_move() {
        let board = open_duel();
        let agent = MinimaxAgent::new(2);
        let mv = agent.choose_move(&board, SnakeId(0));
        assert!(is_reasonable(&board, SnakeId(0), mv));
    }

    #[test]
    fn legacy_policy_still_selects_a_reasonable_move() {
        let board = open_duel();
        let agent = MinimaxAgent::with_policy("legacy", 2, MinimaxPolicy::Legacy);
        let mv = agent.choose_move(&board, SnakeId(0));
        assert!(is_reasonable(&board, SnakeId(0), mv));
    }
}
