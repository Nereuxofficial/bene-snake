use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};

use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::CellIndex,
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        Action, FoodGettableGame, FoodQueryableGame, HazardQueryableGame, HeadGettableGame,
        HealthGettableGame, LengthGettableGame, Move, MoveArray, NeckQueryableGame,
        PositionGettableGame, RandomReasonableMovesGame, ReasonableMovesGame, SizeDeterminableGame,
        SnakeId, VictorDeterminableGame,
    },
    wire_representation::Position,
};
use rand::{Rng, RngExt, seq::IndexedRandom};

use crate::eval::evaluate_board;
use tracing::info;

const MAX_ROLLOUT_DEPTH: u32 = 64;
const MAX_TREE_DEPTH: usize = 64;
// Terminal rewards and UCB must use the same scale. See experiments/reward-scale/.
const WIN_REWARD: u32 = 1000;
const LEAF_SCORE_HALF_REWARD: u32 = 1000;
const OPPONENT_UNIFORM_PERCENT: u32 = 20;

fn leaf_reward(score: u16) -> u32 {
    let score = u32::from(score);
    // Preserve heuristic ordering without letting a live leaf equal a terminal win.
    (WIN_REWARD * score / (score + LEAF_SCORE_HALF_REWARD)).clamp(1, WIN_REWARD - 1)
}

/// A cached per-snake policy built from the state before any moves are chosen.
struct MovePolicy {
    weights: [u8; 4],
    /// Bit `Move::as_index()` is set for each reasonable move whose destination can also
    /// be entered by a living opponent that is at least as long after the move. The
    /// simulator kills every snake tied for the longest new head, so an equal or longer
    /// opponent sharing our destination means we lose that head-to-head.
    losing_head_contests: u8,
}

/// Build one cached small-integer policy per snake from the state before any moves are chosen.
/// The score is deliberately simple: all legal moves start with support, then safe mobility and
/// urgent food progress add weight, while likely losing head contests reduce it. The
/// `losing_head_contests` mask is the same tactical check the tree uses to prune our own moves.
fn opponent_move_policy(
    board: &CellBoard4Snakes11x11,
    snake: SnakeId,
    legal: MoveArray,
    all_moves: &ArrayVec<(SnakeId, MoveArray), 4>,
) -> MovePolicy {
    let head = board.get_head_as_position(&snake);
    let health = board.get_health_i64(&snake);
    let length = board.get_length_i64(&snake);
    let food = board.get_all_food_as_positions();
    let nearest_food = food.iter().map(|pos| manhattan(head, *pos)).min();
    let food_is_urgent = health <= 50;
    let mut policy = MovePolicy {
        weights: [0; 4],
        losing_head_contests: 0,
    };

    for mv in legal {
        let destination = head.add_vec(mv.to_vector());
        let off_board = board.off_board(destination);
        let native = (!off_board).then(|| board.native_from_position(destination));
        // The native position must only be inspected after validating bounds: when every
        // direction is blocked the simulator's conventional fallback is Up, even off-board.
        let has_food = native.as_ref().is_some_and(|pos| board.is_food(pos));
        let immediate_hazard_death = native.as_ref().is_some_and(|pos| {
            board.is_hazard(pos) && health <= 1 + i64::from(board.get_hazard_damage()) && !has_food
        });
        let starvation_death = health <= 1 && !has_food;
        let reverses_into_neck = native
            .as_ref()
            .is_some_and(|pos| board.is_neck(&snake, pos));
        let dies_immediately =
            off_board || immediate_hazard_death || starvation_death || reverses_into_neck;

        let mut weight = 4u8;
        if dies_immediately {
            weight = 1;
        } else {
            let mobility = board
                .free_neighbors(CellIndex::new(destination, board.get_width() as u8))
                .count() as u8;
            weight += mobility.min(4) * 2;
            if food_is_urgent && has_food {
                weight += 8;
            } else if food_is_urgent
                && nearest_food.is_some_and(|distance| {
                    food.iter()
                        .map(|pos| manhattan(destination, *pos))
                        .min()
                        .is_some_and(|next| next < distance)
                })
            {
                weight += 3;
            }
        }

        // The simulator compares lengths after the move, including growth. Two snakes
        // entering the same square both grow together, so comparing the pre-move lengths
        // here is equivalent. Food still matters because `is_feasible_destination` only
        // keeps a low-health opponent alive when the shared square is food.
        let shared_losing_destination = all_moves.iter().any(|(other, other_legal)| {
            if *other == snake || board.get_length_i64(other) < length {
                return false;
            }
            let other_health = board.get_health_i64(other);
            let other_head = board.get_head_as_position(other);
            other_legal.iter().any(|other_mv| {
                let other_destination = other_head.add_vec(other_mv.to_vector());
                other_destination == destination
                    && is_feasible_destination(board, *other, other_destination, other_health)
            })
        });
        if shared_losing_destination {
            policy.losing_head_contests |= 1 << mv.as_index();
            weight = (weight / 4).max(1);
        }
        policy.weights[mv.as_index()] = weight;
    }
    policy
}

/// Keep only the reasonable moves that do not lose a head-to-head. When every move is
/// contested, fall back to the full reasonable set so search retains a nonempty candidate
/// set and can compare those unavoidable risks.
fn pruned_tree_moves(legal: MoveArray, losing_head_contests: u8) -> MoveArray {
    let kept: MoveArray = legal
        .into_iter()
        .filter(|mv| losing_head_contests & (1 << mv.as_index()) == 0)
        .collect();
    if kept.is_empty() { legal } else { kept }
}

#[cfg(test)]
fn opponent_move_weights(
    board: &CellBoard4Snakes11x11,
    snake: SnakeId,
    legal: MoveArray,
    all_moves: &ArrayVec<(SnakeId, MoveArray), 4>,
) -> [u8; 4] {
    opponent_move_policy(board, snake, legal, all_moves).weights
}

fn is_feasible_destination(
    board: &CellBoard4Snakes11x11,
    snake: SnakeId,
    destination: Position,
    health: i64,
) -> bool {
    if board.off_board(destination) {
        return false;
    }
    let native = board.native_from_position(destination);
    if board.is_neck(&snake, &native) {
        return false;
    }
    let can_enter = board
        .free_neighbors(CellIndex::new(
            board.get_head_as_position(&snake),
            board.get_width() as u8,
        ))
        .any(|neighbor| neighbor == native);
    let food = board.is_food(&native);
    let hazard_damage = if board.is_hazard(&native) {
        board.get_hazard_damage()
    } else {
        0
    };
    can_enter && (food || health > 1 + i64::from(hazard_damage))
}

fn manhattan(left: Position, right: Position) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y)
}

fn sample_opponent_move(moves: MoveArray, weights: [u8; 4], rng: &mut impl Rng) -> Move {
    // A 20% uniform component keeps every reasonable move possible; the rest follows the cached
    // weighted heuristic. This avoids a softmax and keeps sampling allocation-free.
    if rng.random_range(0..100) < OPPONENT_UNIFORM_PERCENT {
        return moves.choose(rng).copied().unwrap_or(Move::Up);
    }
    let total_weight: u32 = moves
        .iter()
        .map(|mv| u32::from(weights[mv.as_index()]))
        .sum();
    if total_weight == 0 {
        return moves.choose(rng).copied().unwrap_or(Move::Up);
    }
    let mut sample = rng.random_range(0..total_weight);
    for mv in moves {
        let weight = u32::from(weights[mv.as_index()]);
        if sample < weight {
            return mv;
        }
        sample -= weight;
    }
    Move::Up
}

#[derive(Default)]
pub struct SearchDepthStats {
    pub iterations: u64,
    pub max_tree_depth: usize,
    pub tree_depth_limit_hits: u64,
    pub max_rollout_depth: u32,
    pub rollout_depth_limit_hits: u64,
}

#[derive(Default)]
struct MoveStats {
    visits: AtomicU32,
    reward: AtomicU64,
}

/// A board state before all snakes choose their next moves.
pub struct Node {
    board: CellBoard4Snakes11x11,
    children: Mutex<BTreeMap<Action<4>, Arc<Node>>>,
    visits: AtomicU32,
    own_moves: [MoveStats; 4],
    move_cache: OnceLock<NodeMoveCache>,
}

struct NodeMoveCache {
    moves: ArrayVec<(SnakeId, MoveArray), 4>,
    policies: [[u8; 4]; 4],
    /// Fixed-size, per-snake tree candidate lists with losing head-to-head contests
    /// removed. Only `you` ever reads its entry; opponents keep sampling `moves`.
    tree_moves: [MoveArray; 4],
}

impl Node {
    pub fn new_root(board: CellBoard4Snakes11x11) -> Self {
        Self {
            board,
            children: Mutex::new(BTreeMap::new()),
            visits: AtomicU32::new(0),
            own_moves: std::array::from_fn(|_| MoveStats::default()),
            move_cache: OnceLock::new(),
        }
    }

    fn move_cache(&self) -> &NodeMoveCache {
        self.move_cache.get_or_init(|| {
            let moves = self.board.reasonable_moves_for_each_snake();
            let mut policies = [[0; 4]; 4];
            let mut tree_moves = [MoveArray::new(); 4];
            for (id, legal) in &moves {
                let policy = opponent_move_policy(&self.board, *id, *legal, &moves);
                policies[id.as_usize()] = policy.weights;
                tree_moves[id.as_usize()] = pruned_tree_moves(*legal, policy.losing_head_contests);
            }
            NodeMoveCache {
                moves,
                policies,
                tree_moves,
            }
        })
    }

    pub fn get_depth(&self) -> u32 {
        self.children
            .lock()
            .unwrap()
            .values()
            .map(|child| child.get_depth() + 1)
            .max()
            .unwrap_or(0)
    }

    /// The full reasonable move set, shared with opponents and rollouts. The tree no
    /// longer selects from this directly; it is the fallback when pruning contests all moves.
    #[cfg(test)]
    fn legal_own_moves(&self, you: SnakeId) -> Option<battlesnake_game_types::types::MoveArray> {
        self.move_cache()
            .moves
            .iter()
            .copied()
            .find(|(id, _)| *id == you)
            .map(|(_, moves)| moves)
    }

    /// Our tree candidate moves: reasonable moves minus losing head-to-head contests.
    /// Returns `None` only when `you` has no reasonable moves (for example, a dead snake).
    fn tree_own_moves(&self, you: SnakeId) -> Option<MoveArray> {
        let cache = self.move_cache();
        cache
            .moves
            .iter()
            .any(|(id, _)| *id == you)
            .then(|| cache.tree_moves[you.as_usize()])
    }

    fn select_own_move(&self, you: SnakeId, exploration: f64) -> Option<Move> {
        let moves = self.tree_own_moves(you)?;
        let parent_visits = self.visits.load(Ordering::Relaxed) as f64;
        moves.into_iter().max_by(|left, right| {
            let value = |mv: &Move| {
                let stats = &self.own_moves[mv.as_index()];
                let visits = stats.visits.load(Ordering::Relaxed) as f64;
                if visits == 0.0 {
                    return f64::INFINITY;
                }
                let mean =
                    stats.reward.load(Ordering::Relaxed) as f64 / (visits * f64::from(WIN_REWARD));
                mean + exploration * ((parent_visits + 1.0).ln() / visits).sqrt()
            };
            value(left).total_cmp(&value(right))
        })
    }

    /// Choose only bene-snake's move. Opponent responses are averaged through visits.
    /// Only uncontested tree candidates are considered.
    pub fn best_move(&self, you: SnakeId) -> Option<Move> {
        let moves = self.tree_own_moves(you)?;
        moves.into_iter().max_by(|left, right| {
            let stats = |mv: &Move| &self.own_moves[mv.as_index()];
            let left_visits = stats(left).visits.load(Ordering::Relaxed);
            let right_visits = stats(right).visits.load(Ordering::Relaxed);
            left_visits.cmp(&right_visits).then_with(|| {
                let mean = |mv: &Move, visits: u32| {
                    stats(mv).reward.load(Ordering::Relaxed) as f64 / visits.max(1) as f64
                };
                mean(left, left_visits).total_cmp(&mean(right, right_visits))
            })
        })
    }

    fn sample_joint_action(
        &self,
        you: SnakeId,
        own_move: Move,
        rng: &mut impl Rng,
    ) -> ArrayVec<(SnakeId, Move), 4> {
        let cache = self.move_cache();
        cache
            .moves
            .iter()
            .map(|(id, moves)| {
                let mv = if *id == you {
                    own_move
                } else {
                    sample_opponent_move(*moves, cache.policies[id.as_usize()], rng)
                };
                (*id, mv)
            })
            .collect()
    }

    fn child_for_action(
        &self,
        action: &[(SnakeId, Move)],
    ) -> (Option<Arc<Node>>, CellBoard4Snakes11x11, bool) {
        let key = Action::collect_from(action.iter());
        if let Some(child) = self.children.lock().unwrap().get(&key) {
            return (Some(Arc::clone(child)), child.board, false);
        }

        let next_board = self.board.simulate_single_action(action).1;
        let max_children = 3 + 2 * (self.visits.load(Ordering::Relaxed) as f64).sqrt() as usize;
        let mut children = self.children.lock().unwrap();
        if children.len() >= max_children {
            return (None, next_board, false);
        }

        let child = Arc::new(Node::new_root(next_board));
        children.insert(key, Arc::clone(&child));
        (Some(child), next_board, true)
    }

    fn record(&self, own_move: Move, result: u32) {
        let stats = &self.own_moves[own_move.as_index()];
        stats.reward.fetch_add(result as u64, Ordering::Relaxed);
        stats.visits.fetch_add(1, Ordering::Relaxed);
        self.visits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rollout(&self, you: &SnakeId, stats: &mut SearchDepthStats) -> u32 {
        rollout_from(self.board, you, stats)
    }
}

fn rollout_from(
    mut board: CellBoard4Snakes11x11,
    you: &SnakeId,
    stats: &mut SearchDepthStats,
) -> u32 {
    let mut rng = rand::rng();
    let mut moves = ArrayVec::<(SnakeId, Move), 4>::new();
    let mut depth = 0;

    while !board.is_over() && board.get_health(you) > 0 && depth < MAX_ROLLOUT_DEPTH {
        moves.clear();
        board
            .random_reasonable_move_for_each_snake(&mut rng)
            .collect_into(&mut moves);
        board = board.simulate_single_action(&moves).1;
        depth += 1;
    }

    stats.max_rollout_depth = stats.max_rollout_depth.max(depth);
    if depth == MAX_ROLLOUT_DEPTH && !board.is_over() && board.get_health(you) > 0 {
        stats.rollout_depth_limit_hits += 1;
    }

    if board.get_health(you) == 0 {
        0
    } else if board.is_over() && board.get_winner().is_some_and(|winner| winner == *you) {
        WIN_REWARD
    } else {
        leaf_reward(evaluate_board(&board, you))
    }
}

fn search_iteration(
    root: &Arc<Node>,
    you: &SnakeId,
    rng: &mut impl Rng,
    stats: &mut SearchDepthStats,
) {
    const EXPLORATION: f64 = 1.0;
    stats.iterations += 1;
    let mut path = Vec::with_capacity(16);
    let mut node = Arc::clone(root);
    let result = loop {
        if node.board.is_over() || node.board.get_health(you) == 0 {
            break node.rollout(you, stats);
        }
        if path.len() == MAX_TREE_DEPTH {
            stats.tree_depth_limit_hits += 1;
            break node.rollout(you, stats);
        }
        let Some(own_move) = node.select_own_move(*you, EXPLORATION) else {
            break node.rollout(you, stats);
        };
        let action = node.sample_joint_action(*you, own_move, rng);
        let (child, next_board, newly_expanded) = node.child_for_action(&action);
        path.push((Arc::clone(&node), own_move));

        match child {
            Some(next) if !newly_expanded => node = next,
            _ => break rollout_from(next_board, you, stats),
        }
    };

    stats.max_tree_depth = stats.max_tree_depth.max(path.len());
    for (visited, own_move) in path {
        visited.record(own_move, result);
    }
}

/// Perform one search iteration, mainly useful for profiling the search.
pub fn search_once(root: &Arc<Node>, you: &SnakeId, stats: &mut SearchDepthStats) {
    search_iteration(root, you, &mut rand::rng(), stats);
}

pub fn mcts_search(root: Arc<Node>, you: &SnakeId, stop: Arc<AtomicBool>) {
    let mut rng = rand::rng();
    let mut stats = SearchDepthStats::default();
    while !stop.load(Ordering::Relaxed) {
        search_iteration(&root, you, &mut rng, &mut stats);
    }
    info!(
        iterations = stats.iterations,
        max_tree_depth = MAX_TREE_DEPTH,
        observed_max_tree_depth = stats.max_tree_depth,
        max_tree_depth_reached = stats.max_tree_depth == MAX_TREE_DEPTH,
        tree_depth_cap_truncated_search = stats.tree_depth_limit_hits > 0,
        tree_depth_limit_hits = stats.tree_depth_limit_hits,
        max_rollout_depth = MAX_ROLLOUT_DEPTH,
        observed_max_rollout_depth = stats.max_rollout_depth,
        max_rollout_depth_reached = stats.max_rollout_depth == MAX_ROLLOUT_DEPTH,
        rollout_depth_cap_truncated_search = stats.rollout_depth_limit_hits > 0,
        rollout_depth_limit_hits = stats.rollout_depth_limit_hits,
        "MCTS search depth telemetry"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::wire_representation::Position;
    use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
    use rand::SeedableRng;
    use std::{thread, time::Duration};

    fn turn33() -> (CellBoard4Snakes11x11, SnakeId, SnakeId) {
        let game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid turn-33 fixture");
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid board");
        (board, ids[&game.you.id], ids[&game.board.snakes[0].id])
    }

    fn policy_fixture(
        food: Vec<Position>,
        own_health: u8,
        contested_head: bool,
    ) -> (CellBoard4Snakes11x11, SnakeId, SnakeId) {
        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid fixture");
        let own_wire_id = game.you.id.clone();
        let own_head = Position { x: 5, y: 5 };
        let own_neck = Position { x: 5, y: 4 };
        let other_head = if contested_head {
            Position { x: 5, y: 7 }
        } else {
            Position { x: 2, y: 9 }
        };
        let other_neck = Position {
            x: other_head.x,
            y: other_head.y + 1,
        };
        for snake in &mut game.board.snakes {
            if snake.id == own_wire_id {
                snake.health = i32::from(own_health);
                snake.head = own_head;
                snake.body = vec![own_head, own_neck].into();
            } else {
                snake.head = other_head;
                snake.body = vec![other_head, other_neck].into();
            }
        }
        game.board.food = food;
        game.you.health = i32::from(own_health);
        game.you.head = own_head;
        game.you.body = vec![own_head, own_neck].into();
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid policy board");
        (
            board,
            ids[&own_wire_id],
            ids.values()
                .copied()
                .find(|id| *id != ids[&own_wire_id])
                .unwrap(),
        )
    }

    /// Build an 11x11 board from explicit, already-adjacent body chains. The first
    /// position is the head. `opponents` is a slice of (body, health) specs.
    fn board_from_specs(
        own_body: &[Position],
        own_health: i32,
        opponents: &[(&[Position], i32)],
        food: Vec<Position>,
    ) -> (CellBoard4Snakes11x11, SnakeId, Vec<SnakeId>) {
        use battlesnake_game_types::wire_representation::BattleSnake;

        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid fixture");
        let own_wire_id = game.you.id.clone();
        let template = game
            .board
            .snakes
            .iter()
            .find(|snake| snake.id != own_wire_id)
            .expect("fixture has an opponent")
            .clone();

        let set_body = |snake: &mut BattleSnake, body: &[Position], health: i32| {
            snake.head = body[0];
            snake.body = body.iter().copied().collect();
            snake.health = health;
        };

        set_body(&mut game.you, own_body, own_health);
        let mut snakes = vec![game.you.clone()];
        for (index, (body, health)) in opponents.iter().enumerate() {
            let mut snake = template.clone();
            snake.id = format!("gs_opponent_{index}");
            snake.name = format!("opponent-{index}");
            set_body(&mut snake, body, *health);
            snakes.push(snake);
        }
        game.board.snakes = snakes;
        game.board.food = food;

        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid board");
        let own = ids[&own_wire_id];
        let opponent_ids = (0..opponents.len())
            .map(|index| ids[&format!("gs_opponent_{index}")])
            .collect();
        (board, own, opponent_ids)
    }

    #[test]
    fn equal_length_shared_head_destination_is_pruned() {
        // Own head (5, 5), opponent head (5, 7): both can enter (5, 6). Equal lengths
        // mean the simulator kills both of us, so Up must leave the tree candidate set.
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        assert!(legal.contains(&Move::Up));
        let tree = node.tree_own_moves(you).unwrap();
        assert!(!tree.contains(&Move::Up));
        assert!(tree.contains(&Move::Left));
        assert!(tree.contains(&Move::Right));
        assert!(!tree.as_slice().is_empty());
    }

    #[test]
    fn longer_opponent_shared_head_destination_is_pruned() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(
                &[
                    Position::new(5, 7),
                    Position::new(5, 8),
                    Position::new(5, 9),
                ],
                100,
            )],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let tree = node.tree_own_moves(you).unwrap();
        assert!(!tree.contains(&Move::Up));
        assert!(tree.contains(&Move::Left));
    }

    #[test]
    fn shorter_opponent_shared_head_destination_is_retained() {
        // We are the longer snake, so we win the shared square and keep the move.
        let (board, you, _) = board_from_specs(
            &[
                Position::new(5, 5),
                Position::new(5, 4),
                Position::new(5, 3),
            ],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        assert!(node.tree_own_moves(you).unwrap().contains(&Move::Up));
    }

    #[test]
    fn uncontested_destinations_keep_the_full_reasonable_set() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(2, 9), Position::new(2, 10)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        let tree = node.tree_own_moves(you).unwrap();
        assert_eq!(tree.as_slice(), legal.as_slice());
        assert!(tree.contains(&Move::Up));
    }

    #[test]
    fn food_growth_keeps_a_starving_challenger_in_the_head_to_head() {
        // Equal lengths, opponent at 1 HP, and the shared square is food. Eating keeps
        // the opponent alive and grows both snakes, so the tie still kills us. Without
        // the food the opponent starves and never contests the square.
        let own = [Position::new(5, 5), Position::new(5, 4)];
        let opponent = [Position::new(5, 7), Position::new(5, 8)];
        let shared = Position::new(5, 6);

        let (fed_board, fed_you, fed_opponents) =
            board_from_specs(&own, 100, &[(&opponent, 1)], vec![shared]);
        let fed_node = Node::new_root(fed_board);
        let fed_opponent = fed_opponents[0];
        assert!(
            fed_node
                .tree_own_moves(fed_you)
                .unwrap()
                .contains(&Move::Left)
        );
        assert!(
            !fed_node
                .tree_own_moves(fed_you)
                .unwrap()
                .contains(&Move::Up)
        );

        let (_, fed_next) =
            fed_board.simulate_single_action(&[(fed_you, Move::Up), (fed_opponent, Move::Down)]);
        assert_eq!(
            fed_next.get_health_i64(&fed_you),
            0,
            "both grew to equal length, so both die"
        );
        assert_eq!(fed_next.get_health_i64(&fed_opponent), 0);

        let (hungry_board, hungry_you, hungry_opponents) =
            board_from_specs(&own, 100, &[(&opponent, 1)], Vec::new());
        let hungry_node = Node::new_root(hungry_board);
        assert!(
            hungry_node
                .tree_own_moves(hungry_you)
                .unwrap()
                .contains(&Move::Up)
        );

        let (_, hungry_next) = hungry_board
            .simulate_single_action(&[(hungry_you, Move::Up), (hungry_opponents[0], Move::Down)]);
        assert_eq!(
            hungry_next.get_health_i64(&hungry_opponents[0]),
            0,
            "without food the 1 HP opponent starves"
        );
        assert!(hungry_next.get_health_i64(&hungry_you) > 0);
    }

    #[test]
    fn opponent_that_dies_in_hazard_does_not_prune_shared_destination() {
        use battlesnake_game_types::types::HazardSettableGame;

        let shared = Position::new(5, 6);
        let (template, _, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let opponent_health = i32::from(template.get_hazard_damage()) + 1;

        // Reset to exactly one move's worth of health above lethal hazard damage.
        // The opponent's move to the shared square is geometrically reasonable but dies
        // during per-snake state generation, before head-to-head resolution.
        let (mut board, you, opponents) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], opponent_health)],
            Vec::new(),
        );
        board.set_hazard(board.native_from_position(shared));
        let node = Node::new_root(board);
        assert!(node.tree_own_moves(you).unwrap().contains(&Move::Up));

        let (_, next) =
            board.simulate_single_action(&[(you, Move::Up), (opponents[0], Move::Down)]);
        assert!(next.get_health_i64(&you) > 0);
        assert_eq!(next.get_health_i64(&opponents[0]), 0);
    }

    #[test]
    fn all_contested_moves_fall_back_to_the_full_reasonable_set() {
        // Own head (0, 5) has a spine going down, leaving only Up (0, 6) and Right
        // (1, 5) as reasonable moves (Down is the neck, Left is off-board). Two
        // equal-length opponents can each enter one of them, so naive filtering would
        // leave nothing. The fallback must keep both moves deterministically.
        let (board, you, _) = board_from_specs(
            &[
                Position::new(0, 5),
                Position::new(0, 4),
                Position::new(0, 3),
            ],
            100,
            &[
                (
                    &[
                        Position::new(1, 6),
                        Position::new(2, 6),
                        Position::new(3, 6),
                    ],
                    100,
                ),
                (
                    &[
                        Position::new(1, 4),
                        Position::new(2, 4),
                        Position::new(3, 4),
                    ],
                    100,
                ),
            ],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        assert_eq!(legal.as_slice(), &[Move::Up, Move::Right]);
        let tree = node.tree_own_moves(you).unwrap();
        assert_eq!(tree.as_slice(), legal.as_slice());
        assert_eq!(
            node.tree_own_moves(you).unwrap().as_slice(),
            tree.as_slice()
        );
    }

    #[test]
    fn pruning_only_changes_our_tree_candidates() {
        let (board, you, opponents) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let opponent = opponents[0];
        let full = board.reasonable_moves_for_each_snake();
        let node = Node::new_root(board);
        let cache = node.move_cache();

        // The sampling lists used by opponents and rollouts are untouched by pruning.
        for (id, legal) in &full {
            let cached = cache
                .moves
                .iter()
                .find(|(cached_id, _)| cached_id == id)
                .map(|(_, moves)| moves)
                .unwrap();
            assert_eq!(cached.as_slice(), legal.as_slice());
        }
        assert!(
            !cache.tree_moves[you.as_usize()].contains(&Move::Up),
            "our tree list must be pruned"
        );
        let cached_opponent = cache
            .moves
            .iter()
            .find(|(id, _)| *id == opponent)
            .map(|(_, moves)| moves)
            .unwrap();
        assert!(
            cached_opponent.contains(&Move::Down),
            "the opponent's full move list still contains the contested move"
        );

        // Opponent sampling consumes the same full list with the same policy weights.
        let opponent_legal = full
            .iter()
            .find(|(id, _)| *id == opponent)
            .map(|(_, moves)| *moves)
            .unwrap();
        let mut joint_rng = rand::rngs::SmallRng::seed_from_u64(7);
        let mut expected_rng = rand::rngs::SmallRng::seed_from_u64(7);
        let joint = node.sample_joint_action(you, Move::Left, &mut joint_rng);
        let sampled = joint.iter().find(|(id, _)| *id == opponent).unwrap().1;
        let expected = sample_opponent_move(
            opponent_legal,
            cache.policies[opponent.as_usize()],
            &mut expected_rng,
        );
        assert_eq!(sampled, expected);
    }

    #[test]
    fn best_and_selection_ignore_contested_losing_moves() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        // Even with a decisive reward history, a pruned move is never selected.
        node.record(Move::Up, WIN_REWARD * 10);
        node.record(Move::Left, 1);
        assert_ne!(node.best_move(you), Some(Move::Up));
        assert_ne!(node.select_own_move(you, 1.0), Some(Move::Up));
        assert_eq!(node.best_move(you), Some(Move::Left));
    }

    #[test]
    fn leaf_rewards_stay_between_terminal_outcomes_and_preserve_order() {
        let mut previous = 0;
        for score in 0..=u16::MAX {
            let reward = leaf_reward(score);
            assert!(reward > 0 && reward < WIN_REWARD);
            assert!(reward >= previous);
            previous = reward;
        }
        assert!(leaf_reward(1600) > leaf_reward(800));
    }

    #[test]
    fn one_lucky_win_does_not_dominate_consistently_good_leaves() {
        let (board, you, _) = turn33();
        let node = Node::new_root(board);
        let moves: Vec<_> = node.legal_own_moves(you).unwrap().into_iter().collect();
        assert!(moves.len() >= 2);
        for &mv in &moves {
            for i in 0..1000 {
                let reward = if mv == moves[1] {
                    leaf_reward(800)
                } else if mv == moves[0] && i == 0 {
                    WIN_REWARD
                } else {
                    0
                };
                node.record(mv, reward);
            }
        }
        assert_eq!(node.select_own_move(you, 1.0), Some(moves[1]));
    }

    #[test]
    fn rollout_uses_terminal_rewards_for_winner_and_dead_snake() {
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        game.board.snakes.retain(|snake| snake.id == game.you.id);
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).unwrap();
        let you = ids[&game.you.id];
        let node = Node::new_root(board);
        assert_eq!(
            node.rollout(&you, &mut SearchDepthStats::default()),
            WIN_REWARD
        );
        assert_eq!(
            node.rollout(&SnakeId(3), &mut SearchDepthStats::default()),
            0
        );
    }

    #[test]
    fn opponent_moves_are_sampled_independently_of_our_move() {
        let (board, you, opponent) = turn33();
        let node = Node::new_root(board);
        assert_ne!(opponent, SnakeId(0));
        let mut up_rng = rand::rngs::SmallRng::seed_from_u64(12);
        let mut down_rng = rand::rngs::SmallRng::seed_from_u64(12);
        for _ in 0..100 {
            let up = node.sample_joint_action(you, Move::Up, &mut up_rng);
            let down = node.sample_joint_action(you, Move::Down, &mut down_rng);
            assert_eq!(
                up.iter().find(|(id, _)| *id == opponent),
                down.iter().find(|(id, _)| *id == opponent)
            );
        }
    }

    #[test]
    fn hungry_opponent_policy_favors_food_and_keeps_all_moves_supported() {
        let food_pos = Position { x: 4, y: 5 };
        let (board, snake, _) = policy_fixture(vec![food_pos], 1, false);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = opponent_move_weights(&board, snake, legal, &moves);
        assert!(weights[Move::Left.as_index()] > weights[Move::Right.as_index()]);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));
    }

    #[test]
    fn equal_or_larger_shared_head_square_is_downweighted() {
        let (board, snake, _) = policy_fixture(Vec::new(), 100, true);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = opponent_move_weights(&board, snake, legal, &moves);
        assert!(legal.contains(&Move::Up));
        assert!(weights[Move::Up.as_index()] < weights[Move::Left.as_index()]);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));
    }

    #[test]
    fn no_food_and_empty_move_lists_have_safe_fallbacks() {
        let (board, snake, _) = policy_fixture(Vec::new(), 100, false);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = opponent_move_weights(&board, snake, legal, &moves);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));

        let mut rng = rand::rngs::SmallRng::seed_from_u64(31);
        assert_eq!(
            sample_opponent_move(MoveArray::new(), [0; 4], &mut rng),
            Move::Up
        );
    }

    #[test]
    fn uniform_component_keeps_zero_weight_moves_sampleable() {
        let moves: MoveArray = [Move::Up, Move::Left, Move::Right].into_iter().collect();
        let mut weights = [0; 4];
        weights[Move::Up.as_index()] = 20;
        let mut rng = rand::rngs::SmallRng::seed_from_u64(44);
        let mut counts = [0; 4];
        for _ in 0..6000 {
            let mv = sample_opponent_move(moves, weights, &mut rng);
            assert!(moves.contains(&mv));
            counts[mv.as_index()] += 1;
        }
        assert!(counts[Move::Up.as_index()] > 4500);
        assert!((200..650).contains(&counts[Move::Left.as_index()]));
        assert!((200..650).contains(&counts[Move::Right.as_index()]));
    }

    #[test]
    fn infeasible_challengers_and_short_snake_reversals_are_not_safe() {
        use battlesnake_game_types::types::HazardSettableGame;
        let (mut board, snake, _) = policy_fixture(Vec::new(), 100, false);
        let head = board.get_head_as_position(&snake);
        let neck = head.add_vec(Move::Down.to_vector());
        assert!(!is_feasible_destination(&board, snake, neck, 100));
        assert!(!is_feasible_destination(
            &board,
            snake,
            Position::new(5, 11),
            100
        ));
        let destination = head.add_vec(Move::Left.to_vector());
        let native = board.native_from_position(destination);
        board.set_hazard(native);
        let damage = i64::from(board.get_hazard_damage());
        assert!(!is_feasible_destination(
            &board,
            snake,
            destination,
            damage + 1
        ));
        assert!(is_feasible_destination(
            &board,
            snake,
            destination,
            damage + 2
        ));
        let node = Node::new_root(board);
        let weights = &node.move_cache().policies[snake.as_usize()];
        assert_eq!(weights[Move::Down.as_index()], 1);
    }

    #[test]
    fn policy_weights_handle_off_board_forced_fallback() {
        // A fallback list can contain an off-board move; use a top-edge snake from a fixture.
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        game.board.snakes.retain(|s| s.id == game.you.id);
        game.you.head = Position::new(5, 10);
        game.you.body = vec![game.you.head, Position::new(5, 9)].into();
        game.board.snakes[0] = game.you.clone();
        let ids = build_snake_id_map(&game);
        let edge_board = game.as_cell_board(&ids).unwrap();
        let snake = ids[&game.you.id];
        let fallback: MoveArray = [Move::Up].into_iter().collect();
        let all_moves = [(snake, fallback)].into_iter().collect();
        let weights = opponent_move_weights(&edge_board, snake, fallback, &all_moves);
        assert_eq!(weights[Move::Up.as_index()], 1);
    }

    #[test]
    fn move_statistics_aggregate_across_opponent_responses() {
        let (board, you, _) = turn33();
        let node = Node::new_root(board);
        node.record(Move::Up, 1000);
        node.record(Move::Up, 0);
        node.record(Move::Right, 1000);
        assert_eq!(
            node.own_moves[Move::Up.as_index()]
                .visits
                .load(Ordering::Relaxed),
            2
        );
        assert_eq!(
            node.own_moves[Move::Up.as_index()]
                .reward
                .load(Ordering::Relaxed),
            1000
        );
        assert_eq!(node.best_move(you), Some(Move::Up));
    }

    #[test]
    fn search_stops_and_produces_a_legal_move() {
        let (board, you, _) = turn33();
        let root = Arc::new(Node::new_root(board));
        let stop = Arc::new(AtomicBool::new(false));
        let search_root = Arc::clone(&root);
        let search_stop = Arc::clone(&stop);
        let search = thread::spawn(move || mcts_search(search_root, &you, search_stop));
        thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        search.join().unwrap();
        assert!(root.visits.load(Ordering::Relaxed) > 0);
        assert!(
            root.legal_own_moves(you)
                .unwrap()
                .contains(&root.best_move(you).unwrap())
        );
    }

    #[test]
    #[ignore = "manual replay diagnostic"]
    fn diagnose_turn33_food_choice() {
        let (board, you, _) = turn33();
        for _ in 0..8 {
            let root = Arc::new(Node::new_root(board));
            let stop = Arc::new(AtomicBool::new(false));
            let search_root = Arc::clone(&root);
            let search_stop = Arc::clone(&stop);
            let search = thread::spawn(move || mcts_search(search_root, &you, search_stop));
            thread::sleep(Duration::from_millis(400));
            stop.store(true, Ordering::Relaxed);
            search.join().unwrap();
            println!("{:?}", root.best_move(you));
        }
    }
}
