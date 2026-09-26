use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};

use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        Action, HealthGettableGame, Move, RandomReasonableMovesGame, ReasonableMovesGame, SnakeId,
        VictorDeterminableGame,
    },
};
use rand::{Rng, seq::IndexedRandom};

use crate::eval::evaluate_board;
use tracing::info;

const MAX_ROLLOUT_DEPTH: u32 = 32;
const MAX_TREE_DEPTH: usize = 64;
// Terminal rewards and UCB must use the same scale. See experiments/reward-scale/.
const WIN_REWARD: u32 = 1000;
const LEAF_SCORE_HALF_REWARD: u32 = 1000;

fn leaf_reward(score: u16) -> u32 {
    let score = u32::from(score);
    // Preserve heuristic ordering without letting a live leaf equal a terminal win.
    (WIN_REWARD * score / (score + LEAF_SCORE_HALF_REWARD)).clamp(1, WIN_REWARD - 1)
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
}

impl Node {
    pub fn new_root(board: CellBoard4Snakes11x11) -> Self {
        Self {
            board,
            children: Mutex::new(BTreeMap::new()),
            visits: AtomicU32::new(0),
            own_moves: std::array::from_fn(|_| MoveStats::default()),
        }
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

    fn legal_own_moves(&self, you: SnakeId) -> Option<battlesnake_game_types::types::MoveArray> {
        self.board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .find(|(id, _)| *id == you)
            .map(|(_, moves)| moves)
    }

    fn select_own_move(&self, you: SnakeId, exploration: f64) -> Option<Move> {
        let moves = self.legal_own_moves(you)?;
        let parent_visits = self.visits.load(Ordering::Relaxed) as f64;
        moves.into_iter().max_by(|left, right| {
            let value = |mv: &Move| {
                let stats = &self.own_moves[mv.as_index()];
                let visits = stats.visits.load(Ordering::Relaxed) as f64;
                if visits == 0.0 {
                    return f64::INFINITY;
                }
                let mean = stats.reward.load(Ordering::Relaxed) as f64
                    / (visits * f64::from(WIN_REWARD));
                mean + exploration * ((parent_visits + 1.0).ln() / visits).sqrt()
            };
            value(left).total_cmp(&value(right))
        })
    }

    /// Choose only bene-snake's move. Opponent responses are averaged through visits.
    pub fn best_move(&self, you: SnakeId) -> Option<Move> {
        let moves = self.legal_own_moves(you)?;
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
        self.board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .map(|(id, moves)| {
                let mv = if id == you {
                    own_move
                } else {
                    *moves.choose(rng).expect("living snake has a move")
                };
                (id, mv)
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
        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .unwrap();
        game.board.snakes.retain(|snake| snake.id == game.you.id);
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).unwrap();
        let you = ids[&game.you.id];
        let node = Node::new_root(board);
        assert_eq!(node.rollout(&you, &mut SearchDepthStats::default()), WIN_REWARD);
        assert_eq!(node.rollout(&SnakeId(3), &mut SearchDepthStats::default()), 0);
    }

    #[test]
    fn opponent_moves_are_sampled_independently_of_our_move() {
        let (board, you, opponent) = turn33();
        let node = Node::new_root(board);
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
