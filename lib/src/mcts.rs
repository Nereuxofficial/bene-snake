use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        Action, HealthGettableGame, Move, RandomReasonableMovesGame, ReasonableMovesGame,
        SimulableGame, SimulatorInstruments, SnakeId, VictorDeterminableGame,
    },
};
use dashmap::DashMap;
use once_cell::sync::Lazy;

use crate::eval::evaluate_board;
use crate::non_pushable_queue::NonPushableQueue;
use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// Global evaluation cache shared across all MCTS nodes
/// Uses DashMap for lock-free concurrent access
static EVAL_CACHE: Lazy<DashMap<u64, u16>> = Lazy::new(|| DashMap::with_capacity(100_000));

/// Fast hash function for board state to enable caching
/// Uses a stable hash based on board memory representation
#[inline]
fn hash_board(board: &CellBoard4Snakes11x11) -> u64 {
    let mut hasher = AHasher::default();
    // Hash the raw bytes of the board structure for fast, stable hashing
    // This is safe because we're just reading the bytes
    let ptr = board as *const CellBoard4Snakes11x11 as *const u8;
    let size = std::mem::size_of::<CellBoard4Snakes11x11>();
    unsafe {
        let slice = std::slice::from_raw_parts(ptr, size);
        slice.hash(&mut hasher);
    }
    hasher.finish()
}

/// Iterator that generates all possible combinations of moves for each snake (Cartesian product)
struct MoveCombinationIterator {
    snake_moves: Vec<(SnakeId, Vec<Move>)>,
    indices: Vec<usize>,
    exhausted: bool,
}

impl MoveCombinationIterator {
    fn new(snake_moves: Vec<(SnakeId, Vec<Move>)>) -> Self {
        // Empty input or any snake with no moves means we'll yield once then stop
        let exhausted = snake_moves.iter().any(|(_, moves)| moves.is_empty());
        let indices = vec![0; snake_moves.len()];
        Self {
            snake_moves,
            indices,
            exhausted,
        }
    }
}

impl Iterator for MoveCombinationIterator {
    type Item = Vec<(SnakeId, Move)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        }

        // Handle empty snake_moves case: yield one empty combination
        if self.snake_moves.is_empty() {
            self.exhausted = true;
            return Some(vec![]);
        }

        // Build current combination
        let combination: Vec<(SnakeId, Move)> = self
            .snake_moves
            .iter()
            .zip(&self.indices)
            .map(|((snake_id, moves), &idx)| (*snake_id, moves[idx]))
            .collect();

        // Increment indices (like counting in mixed-radix)
        let mut carry = true;
        for i in (0..self.indices.len()).rev() {
            if carry {
                self.indices[i] += 1;
                if self.indices[i] >= self.snake_moves[i].1.len() {
                    self.indices[i] = 0;
                } else {
                    carry = false;
                }
            }
        }

        // If we still have carry, we've exhausted all combinations
        if carry {
            self.exhausted = true;
        }

        Some(combination)
    }
}

/// Generate an iterator over all possible combinations of moves for each snake (Cartesian product)
fn generate_move_combinations(
    snake_moves: Vec<(SnakeId, Vec<Move>)>,
) -> impl Iterator<Item = Vec<(SnakeId, Move)>> {
    MoveCombinationIterator::new(snake_moves)
}

pub struct Node {
    parent_node: Weak<Node>,
    board: CellBoard4Snakes11x11,
    next_nodes: Mutex<BTreeMap<Action<4>, Arc<Node>>>,
    possible_moves: NonPushableQueue<Vec<(SnakeId, Move)>>,
    wins: AtomicU32,
    visits: AtomicU32,
    board_hash: u64, // Cache the board hash for quick lookups
}
#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}
impl Node {
    pub fn new_root(board: CellBoard4Snakes11x11) -> Self {
        Self::new_child(Weak::new(), board)
    }
    pub fn new_child(parent: Weak<Node>, board: CellBoard4Snakes11x11) -> Self {
        let snake_moves: Vec<_> = board.reasonable_moves_for_each_snake().collect();
        let move_combinations = generate_move_combinations(snake_moves);
        let board_hash = hash_board(&board);

        Node {
            parent_node: parent,
            board,
            next_nodes: Mutex::new(BTreeMap::new()),
            possible_moves: NonPushableQueue::new_from_iterator(move_combinations.into_iter()),
            wins: AtomicU32::new(0),
            visits: AtomicU32::new(0),
            board_hash,
        }
    }
    pub fn get_depth(&self) -> u32 {
        self.next_nodes
            .lock()
            .unwrap()
            .values()
            .map(|n| n.get_depth() + 1)
            .max()
            .unwrap_or(0)
    }
    pub fn best_child(&self, c: f32) -> Option<(Action<4>, Arc<Node>)> {
        // Cache parent visits to avoid repeated atomic loads during iteration
        let parent_visits = self.visits.load(Ordering::Relaxed) as f32;

        // Collect entries quickly to minimize lock duration
        let children: Vec<_> = {
            let lock = self.next_nodes.lock().unwrap();
            lock.iter()
                .map(|(action, node)| (*action, node.clone()))
                .collect()
        };

        // Compute UCB1 scores without holding the lock
        children.into_iter().max_by(|(_, node1), (_, node2)| {
            Self::ucb1_from_ref(node1, c, parent_visits).total_cmp(&Self::ucb1_from_ref(
                node2,
                c,
                parent_visits,
            ))
        })
    }
    pub fn is_fully_expanded(&self) -> bool {
        self.possible_moves.is_empty()
    }

    /// Progressive widening: determine if we should expand more children
    /// based on the number of visits to this node
    /// Formula: k * visits^alpha where k=2, alpha=0.5 (square root)
    /// This limits early branching and allows more exploitation before exploration
    pub fn should_expand_more(&self) -> bool {
        if self.is_fully_expanded() {
            return false;
        }

        let visits = self.visits.load(Ordering::Relaxed);
        let num_children = self.next_nodes.lock().unwrap().len();

        // Progressive widening: allow 3 + 2*sqrt(visits) children
        // Start with 3 children minimum to ensure we explore multiple options early
        let max_children = 3 + (2.0 * (visits as f32).sqrt()) as usize;

        num_children < max_children
    }
    pub fn expand(self: Arc<Self>, _you: &SnakeId) -> bool {
        let Some(moves) = self.possible_moves.pop_front() else {
            return false;
        };

        // Convert moves in-place to avoid intermediate allocation
        let moves_for_simulation: Vec<_> = moves.into_iter().map(|(sid, mv)| (sid, [mv])).collect();

        if let Some((action, next_board)) = self
            .board
            .simulate_with_moves(&Instr, &moves_for_simulation)
            .next()
        {
            let node = Self::new_child(Arc::downgrade(&self), next_board);
            let mut next_nodes_lock = self.next_nodes.lock().unwrap();
            next_nodes_lock.insert(action, Arc::new(node));
            return true;
        }
        false
    }

    pub fn ucb1(self: Arc<Self>, c: f32, visits_to_parent: f32) -> f32 {
        Self::ucb1_from_ref(self, c, visits_to_parent)
    }
    pub fn ucb1_from_ref(node: impl AsRef<Self>, c: f32, visits_to_parent: f32) -> f32 {
        let reference = node.as_ref();
        let visits = reference.visits.load(Ordering::Relaxed);
        if visits == 0 {
            return f32::INFINITY; // Unvisited nodes should be explored first
        }

        // Normalize wins by visits to get average score (since rollout returns 0-1000)
        let avg_score = (reference.wins.load(Ordering::Relaxed) as f32) / (visits as f32);

        // UCB1 formula: avg_score + c * sqrt(ln(parent_visits) / visits)
        avg_score.algebraic_add(
            c.algebraic_mul(visits_to_parent.ln().sqrt().algebraic_div(visits as f32)),
        )
    }
    /// Get cached evaluation score for this node's board state
    /// Returns cached value if available, otherwise computes and caches it
    #[inline]
    fn get_cached_eval(&self, you: &SnakeId) -> u16 {
        // Try to get from cache first
        if let Some(score) = EVAL_CACHE.get(&self.board_hash) {
            return *score;
        }

        // Compute evaluation
        let score = evaluate_board(&self.board, you);

        // Cache it for future use
        EVAL_CACHE.insert(self.board_hash, score);

        score
    }

    /// Perform a random rollout with depth limit and evaluation-based scoring
    /// Returns a score normalized to 0-1000 range for better granularity
    pub fn rollout(self: Arc<Self>, you: &SnakeId) -> u32 {
        const MAX_ROLLOUT_DEPTH: u32 = 100; // Limit depth to prevent extremely long simulations

        let mut rng = rand::rng();
        let mut cur_board = self.board;
        let mut moves = Vec::with_capacity(4);
        let mut depth = 0;

        while !cur_board.is_over() && depth < MAX_ROLLOUT_DEPTH {
            moves.clear();
            cur_board
                .random_reasonable_move_for_each_snake(&mut rng)
                .map(|(sid, mv)| (sid, [mv]))
                .collect_into(&mut moves);

            let next_board = cur_board
                .simulate_with_moves(&Instr, &moves)
                .next()
                .unwrap()
                .1;
            cur_board = next_board;
            depth += 1;
        }

        // Return score based on final state
        if cur_board.get_health(you) == 0 {
            0 // We died - worst outcome
        } else if cur_board.is_over() && cur_board.get_winner().is_some_and(|w| w == *you) {
            1000 // We won - best outcome
        } else if cur_board.is_over() {
            0 // We lost
        } else {
            // Game not over but we hit depth limit
            // Use evaluation function to estimate position quality
            // This gives much better signal than binary win/loss
            let eval_score = evaluate_board(&cur_board, you);
            // Normalize eval to 0-1000 range, capped at 1000
            eval_score.min(1000) as u32
        }
    }
    pub fn is_terminal(&self) -> bool {
        self.board.is_over()
    }
    pub fn backpropagate(self: Arc<Self>, result: u32) {
        self.visits
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        // Result is now in 0-1000 range instead of 0-1
        self.wins
            .fetch_add(result, std::sync::atomic::Ordering::AcqRel);
        if let Some(parent) = self.parent_node.upgrade() {
            parent.backpropagate(result)
        }
    }
}

pub fn mcts_search(root_node: Arc<Node>, you: &SnakeId, stop: Arc<AtomicBool>) {
    // Use sqrt(2) as exploration constant (standard UCB1 value)
    // Slightly higher to encourage more exploration
    const EXPLORATION_CONSTANT: f32 = 1.6;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut node = root_node.clone();

        // Selection: traverse tree using UCB1 until we find a node that can be expanded
        while !node.is_terminal() && !node.should_expand_more() {
            if let Some((_, child)) = node.best_child(EXPLORATION_CONSTANT) {
                node = child;
            } else {
                break;
            }
        }

        // Expansion: add a new child if possible (progressive widening)
        if !node.is_terminal() && node.should_expand_more() {
            if node.clone().expand(you) {
                // If we successfully expanded, select the newly added child for rollout
                if let Some((_, child)) = node.best_child(EXPLORATION_CONSTANT) {
                    node = child;
                }
            }
        }

        // Simulation: rollout from current node
        let result = node.clone().rollout(you);

        // Backpropagation: update statistics up the tree
        node.backpropagate(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game as DEGame};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn test_mcts_search_terminates_on_stop_signal() {
        // Load a test fixture
        let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
        let game: DEGame = serde_json::from_str(game_fixture).expect("valid fixture");
        let snake_id_map = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_id_map).expect("valid board");

        let you = SnakeId(0);
        let root_node = Arc::new(Node::new_root(board));
        let stop = Arc::new(AtomicBool::new(false));

        // Clone for the thread
        let stop_clone = Arc::clone(&stop);
        let root_clone = Arc::clone(&root_node);

        // Start mcts_search in a separate thread
        let search_thread = thread::spawn(move || {
            mcts_search(root_clone, &you, stop_clone);
        });

        // Let it run for a bit to ensure it's actually searching
        thread::sleep(Duration::from_millis(100));

        // Signal stop
        let stop_time = Instant::now();
        stop.store(true, Ordering::Relaxed);

        // Wait for the thread to finish with a timeout
        let join_result = search_thread.join();
        let elapsed = stop_time.elapsed();

        // Verify the thread finished successfully
        assert!(
            join_result.is_ok(),
            "mcts_search thread should finish cleanly"
        );

        // Verify it terminated reasonably quickly (within 1 second)
        // If it didn't respect the stop signal, this would timeout or take much longer
        assert!(
            elapsed < Duration::from_secs(1),
            "mcts_search should terminate quickly after stop signal, took {:?}",
            elapsed
        );

        // Verify that at least some work was done
        let visits = root_node.visits.load(Ordering::Acquire);
        assert!(
            visits > 0,
            "mcts_search should have performed at least some iterations"
        );

        println!(
            "Test passed: mcts_search terminated after {} visits in {:?}",
            visits, elapsed
        );
    }

    #[test]
    fn test_mcts_search_immediate_stop() {
        // Test that if stop is already true, mcts_search doesn't do any work
        let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
        let game: DEGame = serde_json::from_str(game_fixture).expect("valid fixture");
        let snake_id_map = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_id_map).expect("valid board");

        let you = SnakeId(0);
        let root_node = Arc::new(Node::new_root(board));
        let stop = Arc::new(AtomicBool::new(true)); // Already set to true

        // Run mcts_search with stop already set
        mcts_search(root_node.clone(), &you, stop);

        // Verify no work was done
        let visits = root_node.visits.load(Ordering::Acquire);
        assert_eq!(
            visits, 0,
            "mcts_search should not perform any iterations when stop is already true"
        );
    }

    #[test]
    fn test_move_combination_iterator() {
        // Test empty input - should yield one empty combination
        let empty_iter = MoveCombinationIterator::new(vec![]);
        let empty_result: Vec<_> = empty_iter.collect();
        assert_eq!(empty_result.len(), 1);
        assert_eq!(empty_result[0], vec![]);

        // Test single snake with multiple moves
        let snake1 = SnakeId(0);
        let single_snake = vec![(snake1, vec![Move::Up, Move::Down, Move::Left])];
        let single_result: Vec<_> = MoveCombinationIterator::new(single_snake).collect();
        assert_eq!(single_result.len(), 3);
        assert_eq!(single_result[0], vec![(snake1, Move::Up)]);
        assert_eq!(single_result[1], vec![(snake1, Move::Down)]);
        assert_eq!(single_result[2], vec![(snake1, Move::Left)]);

        // Test two snakes (Cartesian product)
        let snake2 = SnakeId(1);
        let two_snakes = vec![
            (snake1, vec![Move::Up, Move::Down]),
            (snake2, vec![Move::Left, Move::Right]),
        ];
        let two_result: Vec<_> = MoveCombinationIterator::new(two_snakes).collect();
        assert_eq!(two_result.len(), 4); // 2 x 2 = 4
        assert_eq!(
            two_result[0],
            vec![(snake1, Move::Up), (snake2, Move::Left)]
        );
        assert_eq!(
            two_result[1],
            vec![(snake1, Move::Up), (snake2, Move::Right)]
        );
        assert_eq!(
            two_result[2],
            vec![(snake1, Move::Down), (snake2, Move::Left)]
        );
        assert_eq!(
            two_result[3],
            vec![(snake1, Move::Down), (snake2, Move::Right)]
        );

        // Test three snakes
        let snake3 = SnakeId(2);
        let three_snakes = vec![
            (snake1, vec![Move::Up, Move::Down]),
            (snake2, vec![Move::Left]),
            (snake3, vec![Move::Right, Move::Up]),
        ];
        let three_result: Vec<_> = MoveCombinationIterator::new(three_snakes).collect();
        assert_eq!(three_result.len(), 4); // 2 x 1 x 2 = 4

        // Test snake with no moves (should yield no combinations)
        let no_moves = vec![
            (snake1, vec![Move::Up]),
            (snake2, vec![]), // No moves for this snake
        ];
        let no_moves_result: Vec<_> = MoveCombinationIterator::new(no_moves).collect();
        assert_eq!(no_moves_result.len(), 0);
    }
}
