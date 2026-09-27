use battlesnake_game_types::{
    compact_representation::{CellIndex, standard::CellBoard4Snakes11x11},
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
        MoveArray, NeighborDeterminableGame, ReasonableMovesGame, SimulableGame, SnakeId,
    },
    wire_representation::Position,
};

use lib::Agent;

/// Number of cells on the standard board. Used for allocation-free bookkeeping
/// in the reachable-space flood fill.
const BOARD_CELLS: usize = 11 * 11;

/// Reachable area is only scored up to this many cells (`area.min(AREA_CAP)`),
/// so the flood fill can stop early. Keeping the cap here makes that an exact,
/// behavior-preserving shortcut rather than a heuristic cutoff.
const AREA_CAP: usize = 40;

/// Which scoring policy a [`HeuristicAgent`] should use.
///
/// The variants exist so the two policies can be compared directly in
/// deterministic, seeded duels. The tactical policy is the default because the
/// legacy policy cannot actually steer toward food and assumes opponents move
/// regardless of the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeuristicPolicy {
    /// The original one-ply policy.
    ///
    /// Food distance is measured from the pre-move head, so every candidate move
    /// receives the same food bonus. Candidates are also scored by simulating a
    /// board in which every opponent takes its first legal move, which is not a
    /// property of the state and biases toward whichever move happens to do well
    /// against that arbitrary response.
    Legacy,
    /// Destination-based tactical policy.
    ///
    /// Food, immediate mobility, and reachable space are measured from the
    /// candidate destination, so the policy can distinguish moving toward food
    /// from moving away. A destination that an equal/larger opponent can also
    /// enter this turn is treated as a collision risk, and no opponent move is
    /// assumed to be a particular choice.
    Tactical,
}

/// A heuristic-based agent that uses simple rules to make decisions:
/// - Avoid walls and other snakes
/// - Seek food when health is low (and, tactically, before it is too late)
/// - Prefer moves that maximize available space
pub struct HeuristicAgent {
    name: String,
    /// Health threshold below which the snake prioritizes food
    hunger_threshold: u8,
    policy: HeuristicPolicy,
}

impl HeuristicAgent {
    pub fn new() -> Self {
        Self {
            name: "Heuristic".to_string(),
            hunger_threshold: 30,
            policy: HeuristicPolicy::Tactical,
        }
    }

    pub fn with_name(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::new()
        }
    }

    pub fn with_policy(name: impl Into<String>, policy: HeuristicPolicy) -> Self {
        Self {
            name: name.into(),
            policy,
            ..Self::new()
        }
    }

    pub fn with_hunger_threshold(mut self, threshold: u8) -> Self {
        self.hunger_threshold = threshold;
        self
    }

    /// Original legacy scoring. Kept byte-for-byte in spirit so the two policies
    /// can be compared without the legacy side drifting.
    fn score_move_legacy(&self, board: &CellBoard4Snakes11x11, you: SnakeId, mv: Move) -> i32 {
        let health = board.get_health(&you);
        let length = board.get_length(&you);

        // Simulate the move to see the resulting board
        let moves_for_sim: Vec<_> = board
            .reasonable_moves_for_each_snake()
            .into_iter()
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

        let Some((_, next_board)) = board.simulate_with_moves(&moves_for_sim).next() else {
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
                    let dist = (head_pos.x - food_pos.x).abs() + (head_pos.y - food_pos.y).abs();
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

    /// Destination-based tactical scoring.
    fn score_move_tactical(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        mv: Move,
        snake_moves: &[(SnakeId, MoveArray)],
    ) -> i32 {
        let health = board.get_health(&you) as i32;
        let length = board.get_length(&you) as i32;
        let head_pos = board.get_head_as_position(&you);
        let dest_pos = head_pos.add_vec(mv.to_vector());

        // The candidate destination as a cell index. The candidate comes from
        // the reasonable-move list, so `possible_moves` will contain it; an
        // off-board candidate can only appear in the all-moves fallback.
        let head_idx = board.get_head_as_native_position(&you);
        let Some(dest_idx) = board
            .possible_moves(&head_idx)
            .find(|(m, _)| *m == mv)
            .map(|(_, cell)| cell)
        else {
            return i32::MIN;
        };

        let food_positions = board.get_all_food_as_positions();
        let eats = food_positions.contains(&dest_pos);

        // Starvation is the one immediate death that does not depend on any
        // opponent's choice. Opponent collisions are handled below as risks.
        if !eats && health <= 1 {
            return i32::MIN;
        }

        let next_health = if eats { 100 } else { health - 1 };
        let next_length = length + i32::from(eats);

        let mut score: i32 = 0;

        // Space measured from the destination, not the pre-move head. Immediate
        // free neighbors exclude bodies and heads; the flood fill adds a bounded
        // reachable-region term so pocket moves are not scored like open ones.
        let mobility = board.free_neighbors(dest_idx).count() as i32;
        score += mobility * 12;
        let area = reachable_area(board, dest_idx) as i32;
        score += area * 2;

        // Food value measured from the destination. The pull grows as health
        // falls so a snake eventually commits to food instead of starving.
        // Eating is just the zero-distance case plus a growth bonus, which keeps
        // the ranking monotonic: eat > step closer > step away.
        if let Some(nearest) = food_positions
            .iter()
            .map(|food| manhattan(dest_pos, *food))
            .min()
        {
            let closeness = (12 - nearest).max(0);
            let urgency = (self.hunger_threshold as i32 - health).clamp(0, 30);
            score += closeness * (2 + urgency / 3);
        }
        if eats {
            score += 20;
        }

        // Head-on contests: if an opponent can also enter this destination, the
        // longer post-move snake survives. Compare lengths including a possible
        // growth on that same square. The opponent may choose otherwise, so this
        // is a risk term rather than a forced-loss disqualification.
        for (sid, opp_moves) in snake_moves {
            if *sid == you {
                continue;
            }
            let opp_len = board.get_length(sid) as i32;
            if opp_len == 0 {
                continue;
            }
            let opp_head = board.get_head_as_position(sid);
            for opp_mv in opp_moves {
                let opp_dest = opp_head.add_vec(opp_mv.to_vector());
                if opp_dest != dest_pos {
                    continue;
                }
                let opp_eats = food_positions.contains(&opp_dest);
                let opp_next_length = opp_len + i32::from(opp_eats);
                if next_length > opp_next_length {
                    // We would win the exchange outright.
                    score += 15;
                } else {
                    // Equal lengths eliminate both snakes; a shorter snake loses.
                    score -= 50;
                }
            }
        }

        // Bonus for length (longer is better)
        score += next_length;

        // Penalty for low health
        if next_health < 20 {
            score -= (20 - next_health) * 2;
        }

        score
    }

    fn score_move(
        &self,
        board: &CellBoard4Snakes11x11,
        you: SnakeId,
        mv: Move,
        snake_moves: &[(SnakeId, MoveArray)],
    ) -> i32 {
        match self.policy {
            HeuristicPolicy::Legacy => self.score_move_legacy(board, you, mv),
            HeuristicPolicy::Tactical => self.score_move_tactical(board, you, mv, snake_moves),
        }
    }
}

/// Manhattan distance between two board positions.
fn manhattan(a: Position, b: Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Count cells reachable from `start` by stepping through free cells, saturated
/// at [`AREA_CAP`].
///
/// This treats single tails as free (they vacate) and otherwise ignores tail
/// timing, so it is a bounded approximation of reachable space, not an exact
/// reachability computation. It reuses the same occupancy predicate as move
/// generation and allocates nothing itself; the only allocations are inside the
/// board's boxed neighbor iterator. Saturating at the cap means the caller sees
/// exactly the same `min(area, AREA_CAP)` score while the walk stops at
/// [`AREA_CAP`] cells.
fn reachable_area(board: &CellBoard4Snakes11x11, start: CellIndex<u8>) -> usize {
    let mut visited = [false; BOARD_CELLS];
    let mut stack = [CellIndex::<u8>::from_usize(0); BOARD_CELLS];
    let mut stack_len = 0usize;

    let start_index = start.as_usize();
    if start_index >= BOARD_CELLS {
        return 0;
    }
    visited[start_index] = true;
    stack[stack_len] = start;
    stack_len += 1;

    let mut count = 0usize;
    while stack_len > 0 && count < AREA_CAP {
        stack_len -= 1;
        let cell = stack[stack_len];
        count += 1;
        for neighbor in board.free_neighbors(cell) {
            let index = neighbor.as_usize();
            if index < BOARD_CELLS && !visited[index] {
                visited[index] = true;
                stack[stack_len] = neighbor;
                stack_len += 1;
            }
        }
    }
    count.min(AREA_CAP)
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
        let snake_moves = board.reasonable_moves_for_each_snake();
        let reasonable_moves = snake_moves
            .iter()
            .find(|(sid, _)| *sid == you)
            .map(|(_, moves)| *moves)
            .unwrap_or_else(|| Move::all().into_iter().collect());

        // Score each move and pick the best
        reasonable_moves
            .into_iter()
            .max_by_key(|&mv| self.score_move(board, you, mv, &snake_moves))
            .unwrap_or(Move::Up)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{GameConfig, generate_random_game_with_seed};
    use std::collections::VecDeque;

    /// Build a duel board, then overwrite the snakes and food so a test controls
    /// every relevant cell.
    fn fixture(
        snakes: &[(Position, VecDeque<Position>, i32)],
        food: &[Position],
    ) -> CellBoard4Snakes11x11 {
        let config = GameConfig::duel();
        let mut game = generate_random_game_with_seed(&config, 20260928);
        for (snake, (head, body, health)) in game.board.snakes.iter_mut().zip(snakes) {
            snake.head = *head;
            snake.body = body.clone();
            snake.health = *health;
        }
        game.board.food = food.to_vec();
        game.you = game.board.snakes[0].clone();
        let snake_id_map = battlesnake_game_types::types::build_snake_id_map(&game);
        game.as_cell_board(&snake_id_map).unwrap()
    }

    fn straight_body(head: Position, len: usize, step: (i32, i32)) -> VecDeque<Position> {
        let mut body = VecDeque::new();
        for i in 0..len {
            body.push_back(Position::new(
                head.x - step.0 * i as i32,
                head.y - step.1 * i as i32,
            ));
        }
        body
    }

    fn score(agent: &HeuristicAgent, board: &CellBoard4Snakes11x11, you: SnakeId, mv: Move) -> i32 {
        let snake_moves = board.reasonable_moves_for_each_snake();
        agent.score_move_tactical(board, you, mv, &snake_moves)
    }

    #[test]
    fn default_policy_is_tactical() {
        assert_eq!(HeuristicAgent::new().policy, HeuristicPolicy::Tactical);
    }

    #[test]
    fn tactical_food_bonus_follows_the_candidate_destination() {
        // Our head at (5, 5) and food at (7, 5). Stepping Right lands at (6, 5),
        // one step closer; stepping Left lands at (4, 5), one step farther. The
        // legacy policy measured distance from the pre-move head (always 2), so
        // it could not distinguish these. Opponents and food are placed far from
        // the candidates to keep space terms comparable.
        let head = Position::new(5, 5);
        let body = VecDeque::from([head, head, head]);
        let opponent_head = Position::new(0, 10);
        let opponent_body = straight_body(opponent_head, 3, (0, 1));
        let board = fixture(
            &[(head, body, 20), (opponent_head, opponent_body, 90)],
            &[Position::new(7, 5)],
        );

        let agent = HeuristicAgent::new();
        let you = SnakeId(0);
        let toward = score(&agent, &board, you, Move::Right);
        let away = score(&agent, &board, you, Move::Left);

        assert!(
            toward > away,
            "moving closer to food {toward} should beat moving away {away}"
        );
    }

    #[test]
    fn tactical_prefers_actually_eating_over_merely_approaching() {
        let head = Position::new(5, 5);
        let body = VecDeque::from([head, head, head]);
        let opponent_head = Position::new(0, 10);
        let opponent_body = straight_body(opponent_head, 3, (0, 1));
        // Food directly to the right: Right eats it, every other move only
        // reaches distance 2.
        let board = fixture(
            &[(head, body, 40), (opponent_head, opponent_body, 90)],
            &[Position::new(6, 5)],
        );

        let agent = HeuristicAgent::new();
        let you = SnakeId(0);
        let eat = score(&agent, &board, you, Move::Right);
        for mv in [Move::Up, Move::Down, Move::Left] {
            assert!(
                eat > score(&agent, &board, you, mv),
                "eating {eat} should beat {mv:?}"
            );
        }
    }

    #[test]
    fn tactical_avoids_destination_contestable_by_larger_opponent() {
        // Our head at (5, 5), opponent (length 5, larger) at (3, 5) can step
        // Right into (4, 5). Moving Left into (4, 5) risks losing the head-on;
        // moving Up is not contested and should score higher.
        let head = Position::new(5, 5);
        let body = VecDeque::from([head, head, head]);
        let opponent_head = Position::new(3, 5);
        let opponent_body = straight_body(opponent_head, 4, (1, 0));
        let board = fixture(&[(head, body, 90), (opponent_head, opponent_body, 90)], &[]);

        let agent = HeuristicAgent::new();
        let you = SnakeId(0);
        let contested = score(&agent, &board, you, Move::Left);
        let free = score(&agent, &board, you, Move::Up);

        assert!(
            free > contested,
            "uncontested move {free} should beat a larger-opponent contest {contested}"
        );
    }

    #[test]
    fn starvation_move_is_disqualified_unless_everything_starves() {
        let head = Position::new(5, 5);
        let body = VecDeque::from([head, head, head]);
        let opponent_head = Position::new(0, 10);
        let opponent_body = straight_body(opponent_head, 3, (0, 1));
        // Health 1 with no reachable food: every move starves, so the policy
        // still returns a legal move rather than panicking.
        let board = fixture(&[(head, body, 1), (opponent_head, opponent_body, 90)], &[]);
        let agent = HeuristicAgent::new();
        let mv = agent.choose_move(&board, SnakeId(0));
        assert!(
            board
                .reasonable_moves_for_each_snake()
                .into_iter()
                .find(|(sid, _)| *sid == SnakeId(0))
                .unwrap()
                .1
                .contains(&mv)
        );
    }

    #[test]
    fn legacy_policy_still_selects_a_reasonable_move() {
        let head = Position::new(5, 5);
        let body = VecDeque::from([head, head, head]);
        let opponent_head = Position::new(0, 10);
        let opponent_body = straight_body(opponent_head, 3, (0, 1));
        let board = fixture(
            &[(head, body, 50), (opponent_head, opponent_body, 90)],
            &[Position::new(5, 7)],
        );
        let agent = HeuristicAgent::with_policy("legacy", HeuristicPolicy::Legacy);
        let mv = agent.choose_move(&board, SnakeId(0));
        assert!(
            board
                .reasonable_moves_for_each_snake()
                .into_iter()
                .find(|(sid, _)| *sid == SnakeId(0))
                .unwrap()
                .1
                .contains(&mv)
        );
    }
}
