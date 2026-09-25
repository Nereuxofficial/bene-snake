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
                let mean = stats.reward.load(Ordering::Relaxed) as f64 / (visits * 1000.0);
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

    pub fn rollout(&self, you: &SnakeId) -> u32 {
        rollout_from(self.board, you)
    }
}

fn rollout_from(mut board: CellBoard4Snakes11x11, you: &SnakeId) -> u32 {
    const MAX_ROLLOUT_DEPTH: u32 = 32;
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

    if board.get_health(you) == 0 {
        0
    } else if board.is_over() && board.get_winner().is_some_and(|winner| winner == *you) {
        1000
    } else if board.is_over() {
        0
    } else {
        evaluate_board(&board, you).min(1000) as u32
    }
}

fn search_iteration(root: &Arc<Node>, you: &SnakeId, rng: &mut impl Rng) {
    const EXPLORATION: f64 = 1.0;
    const MAX_TREE_DEPTH: usize = 64;
    let mut path = Vec::with_capacity(16);
    let mut node = Arc::clone(root);
    let result = loop {
        if node.board.is_over() || node.board.get_health(you) == 0 || path.len() == MAX_TREE_DEPTH {
            break node.rollout(you);
        }

        let Some(own_move) = node.select_own_move(*you, EXPLORATION) else {
            break node.rollout(you);
        };
        let action = node.sample_joint_action(*you, own_move, rng);
        let (child, next_board, newly_expanded) = node.child_for_action(&action);
        path.push((Arc::clone(&node), own_move));

        match child {
            Some(next) if !newly_expanded => node = next,
            _ => break rollout_from(next_board, you),
        }
    };

    for (visited, own_move) in path {
        visited.record(own_move, result);
    }
}

/// Perform one search iteration, mainly useful for profiling the search.
pub fn search_once(root: &Arc<Node>, you: &SnakeId) {
    search_iteration(root, you, &mut rand::rng());
}

pub fn mcts_search(root: Arc<Node>, you: &SnakeId, stop: Arc<AtomicBool>) {
    let mut rng = rand::rng();
    while !stop.load(Ordering::Relaxed) {
        search_iteration(&root, you, &mut rng);
    }
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
