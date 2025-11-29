use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU16, Ordering},
    },
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        Action, Move, RandomReasonableMovesGame, ReasonableMovesGame, SimulableGame,
        SimulatorInstruments, SnakeId, VictorDeterminableGame,
    },
};

use crate::eval::evaluate_board;

const MAX_EXPLORE_DEPTH: u8 = 5;

/// Generate all possible combinations of moves for each snake (Cartesian product)
fn generate_move_combinations(snake_moves: Vec<(SnakeId, Vec<Move>)>) -> Vec<Vec<(SnakeId, Move)>> {
    if snake_moves.is_empty() {
        return vec![vec![]];
    }

    let mut result = vec![vec![]];

    for (snake_id, moves) in snake_moves {
        let mut new_result = Vec::new();
        for combination in result {
            for &mv in &moves {
                let mut new_combination = combination.clone();
                new_combination.push((snake_id, mv));
                new_result.push(new_combination);
            }
        }
        result = new_result;
    }

    result
}

pub struct Node {
    parent_node: Weak<Node>,
    board: CellBoard4Snakes11x11,
    next_nodes: Mutex<BTreeMap<Action<4>, Arc<Node>>>,
    possible_moves: Mutex<VecDeque<Vec<(SnakeId, Move)>>>,
    wins: AtomicU16,
    visits: AtomicU16,
}
#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}
impl Node {
    pub fn new_root(board: CellBoard4Snakes11x11, you: &SnakeId) -> Self {
        Self::new_child(Weak::new(), board, you)
    }
    pub fn new_child(parent: Weak<Node>, board: CellBoard4Snakes11x11, you: &SnakeId) -> Self {
        let snake_moves: Vec<_> = board.reasonable_moves_for_each_snake().collect();
        let move_combinations = generate_move_combinations(snake_moves);

        Node {
            parent_node: parent,
            board,
            next_nodes: Mutex::new(BTreeMap::new()),
            possible_moves: Mutex::new(move_combinations.into()),
            wins: AtomicU16::new(0),
            visits: AtomicU16::new(0),
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
        self.next_nodes
            .lock()
            .unwrap()
            .iter()
            .max_by(|(_, node1), (_, node2)| {
                Arc::clone(node1)
                    .ucb1(c, self.visits.load(Ordering::Acquire) as f32)
                    .total_cmp(
                        &Arc::clone(node2).ucb1(c, self.visits.load(Ordering::Acquire) as f32),
                    )
            })
            .map(|(action, node)| (*action, node.clone()))
    }
    pub fn is_fully_expanded(&self) -> bool {
        self.possible_moves.lock().unwrap().is_empty()
    }
    pub fn expand(self: Arc<Self>, you: &SnakeId) {
        let Some(moves) = self.possible_moves.lock().unwrap().pop_front() else {
            return;
        };
        // TODO: Switch to some treemap for next_node
        // Convert Vec<(SnakeId, Move)> into the format needed for simulate_with_moves
        let moves_for_simulation: Vec<(SnakeId, Vec<Move>)> =
            moves.into_iter().map(|(sid, mv)| (sid, vec![mv])).collect();

        let (action, node) = self
            .board
            .simulate_with_moves(
                &Instr,
                moves_for_simulation
                    .iter()
                    .map(|(sid, mvs)| (*sid, mvs.as_slice())),
            )
            .map(|(a, b)| (a, Self::new_child(Arc::downgrade(&self), b, you)))
            .next()
            .unwrap();
        let mut next_nodes_lock = self.next_nodes.lock().unwrap();
        next_nodes_lock.insert(action, Arc::new(node));
    }
    pub fn get_score(&self, you: &SnakeId) -> u16 {
        evaluate_board(&self.board, you)
    }
    pub fn ucb1(self: Arc<Self>, c: f32, visits_to_parent: f32) -> f32 {
        (self.wins.load(Ordering::Acquire) as f32).algebraic_add(
            c.algebraic_mul(
                visits_to_parent
                    .ln()
                    .sqrt()
                    .algebraic_div((self.visits.load(Ordering::Acquire) + 1) as f32),
            ),
        )
    }
    pub fn rollout(self: Arc<Self>, you: &SnakeId) -> u16 {
        let mut rng = rand::rng();
        let mut cur_board = self.board;
        while !cur_board.is_over() {
            let moves = cur_board
                .random_reasonable_move_for_each_snake(&mut rng)
                .map(|(sid, mv)| (sid, [mv]));
            let next_board = cur_board
                .simulate_with_moves(&Instr, moves)
                .next()
                .unwrap()
                .1;
            cur_board = next_board;
        }
        if cur_board.get_winner().is_some_and(|w| w == *you) {
            1
        } else {
            0
        }
    }
    pub fn is_terminal(&self) -> bool {
        self.board.is_over()
    }
    pub fn backpropagate(self: Arc<Self>, result: u16) {
        self.visits
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.wins
            .fetch_add(result, std::sync::atomic::Ordering::AcqRel);
        if let Some(parent) = self.parent_node.upgrade() {
            parent.backpropagate(result)
        }
    }
}

pub fn mcts_search(root_node: Arc<Node>, you: &SnakeId) {
    // TODO: We could look here if we can do this in parallel for different sub-trees by sorting and taking the best few
    let iterations = 500;
    for _ in 0..iterations {
        let mut node = root_node.clone();

        while !node.is_terminal() && node.is_fully_expanded() {
            node = node
                .best_child(1.4)
                .expect("This should be none because we checked the variants here under which this would be None").1;
        }

        if !node.is_terminal() {
            node.clone().expand(you);
        }

        let result = node.clone().rollout(you);

        node.backpropagate(result);
    }
}
