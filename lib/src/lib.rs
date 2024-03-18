use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::{CellBoard, CellBoard4Snakes11x11};
use battlesnake_game_types::compact_representation::CellNum;
use battlesnake_game_types::types::{
    Action, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
    NeighborDeterminableGame, SimulableGame, SimulatorInstruments, SnakeIDGettableGame, SnakeIDMap,
    SnakeId, VictorDeterminableGame, YouDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use std::borrow::Cow;
use std::cmp::{max, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use tracing::info;
use tracing::{info_span, instrument};

pub const DELAY: Duration = Duration::from_millis(150);

pub type GameStates = Arc<Mutex<HashMap<String, SnakeIDMap>>>;

#[instrument]
pub fn decode_state(
    mut text: String,
    game_states: GameStates,
) -> color_eyre::Result<CellBoard4Snakes11x11> {
    let game_states = game_states.lock().unwrap();
    let decoded: Game = unsafe { simd_json::serde::from_str(&mut text) }?;
    let cellboard = decoded
        .as_cell_board(HashMap::get(&game_states, &decoded.game.id).unwrap())
        .unwrap();
    Ok(cellboard)
}

#[instrument(skip(cellboard, start))]
pub fn calc_move(cellboard: CellBoard4Snakes11x11, depth: i64, start: Instant) -> Move {
    let you = cellboard.you_id();
    let snake_ids = cellboard.get_snake_ids();
    let instant_ref = Arc::new(start);
    paranoid_minimax(cellboard, depth, you, Cow::Owned(snake_ids), instant_ref).1
}

/// Given a cellboard, a depth and a SnakeId, this function will search for the best move
/// using a paranoid minimax algorithm.
/// **Inner Workings**
/// We collect all reasonable moves in our current state and simulate each one for our current position.
/// Then we simulate the next opponent moves and pick the least favorable outcome for us.
/// We then simulate the next opponent moves and pick the least favorable outcome for us.
/// Repeat until there are no more opponents left on the current board.
/// In Paranoid Minimax, we want to maximize our own score and minimize the score of our opponents.
#[instrument(skip_all, ret)]
fn paranoid_minimax(
    game: CellBoard4Snakes11x11,
    depth: i64,
    you: &SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
    start: Arc<Instant>,
) -> (f32, Move, i64) {
    if is_won(game, you) {
        return (f32::INFINITY, Move::Down, depth);
    }
    if !game.is_alive(you) {
        return (f32::NEG_INFINITY, Move::Down, depth);
    }
    #[cfg(feature = "bench")]
    if depth == 0 {
        return (evaluate_board(game, &you, snake_ids), Move::Down, depth);
    }
    #[cfg(not(feature = "bench"))]
    if start.elapsed() + DELAY > Duration::from_millis(500) {
        return (evaluate_board(game, you, snake_ids), Move::Down, depth);
    }
    let simulations = game.simulate(&Simulator {}, game.get_snake_ids().to_vec());
    let recursive_scores: Vec<(f32, Move, i64)> = {
        let simulation: Vec<(Action<4>, CellBoard4Snakes11x11)> = simulations.collect();
        let count = simulation.len();
        let mut scores = Vec::with_capacity(count);
        // TODO: rayon is a bit overkill for getting the number of threads, replace it with something else
        let threads = rayon::current_num_threads();
        let mut tasks = Vec::with_capacity(threads);
        // Distribute the work across the threads
        // Split simulations into threads chunks
        let chunk_size = max(count / threads, 1);
        info_span!(
            "Distributing work across threads",
            count = count,
            threads = threads,
            chunk_size = chunk_size
        )
        .in_scope(|| {
            for chunk in simulation.chunks(chunk_size) {
                let chunk = chunk.to_vec();
                let snake_ids_clone = snake_ids.to_vec();
                let start_clone = start.clone();
                let you_clone = *you;
                tasks.push(std::thread::spawn(move || {
                    chunk
                        .into_iter()
                        .map(|(action, b)| {
                            let res = paranoid_minimax_single_threaded(
                                b,
                                depth - 1,
                                you_clone,
                                Cow::Owned(snake_ids_clone.clone()),
                                start_clone.clone(),
                            );
                            (res.0, action.own_move(), res.2)
                        })
                        .collect::<Vec<(f32, Move, i64)>>()
                }));
            }
            // Collect the results
            for task in tasks {
                scores.append(&mut task.join().unwrap());
            }
        });
        scores
    };

    let mut buckets = vec![vec![]; 4];
    for (score, mv, depth) in recursive_scores.iter() {
        buckets[mv.as_index()].push((score, mv, depth));
    }
    info_span!("Finding best move from buckets")
        .in_scope(|| get_best_move_from_buckets(&buckets, depth))
}
fn get_recursive_scores(
    simulations: impl Iterator<Item = (Action<4>, CellBoard4Snakes11x11)>,
    depth: i64,
    you: &SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
    start: Arc<Instant>,
) -> Vec<(f32, Move, i64)> {
    simulations
        .map(|(action, b)| {
            let result = paranoid_minimax_single_threaded(
                b,
                depth - 1,
                *you,
                snake_ids.clone(),
                start.clone(),
            );
            (result.0, action.own_move(), result.2)
        })
        .collect()
}

pub fn paranoid_minimax_single_threaded(
    game: CellBoard4Snakes11x11,
    depth: i64,
    you: SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
    start: Arc<Instant>,
) -> (f32, Move, i64) {
    if is_won(game, &you) {
        return (f32::INFINITY, Move::Down, depth);
    }
    if !game.is_alive(&you) {
        return (f32::NEG_INFINITY, Move::Down, depth);
    }
    #[cfg(feature = "bench")]
    if depth == 0 {
        return (evaluate_board(game, &you, snake_ids), Move::Down, depth);
    }
    #[cfg(not(feature = "bench"))]
    if start.elapsed() + DELAY > Duration::from_millis(500) {
        return (evaluate_board(game, &you, snake_ids), Move::Down, depth);
    }
    let simulations = game.simulate(&Simulator {}, game.get_snake_ids().to_vec());
    let recursive_scores: Vec<(f32, Move, i64)> =
        get_recursive_scores(simulations, depth, &you, snake_ids, start);
    let mut buckets = [vec![], vec![], vec![], vec![]];
    for (score, mv, d) in recursive_scores.iter() {
        buckets[mv.as_index()].push((score, mv, d));
    }
    get_best_move_from_buckets(&buckets, depth)
}

fn get_best_move_from_buckets(
    buckets: &[Vec<(&f32, &Move, &i64)>],
    depth: i64,
) -> (f32, Move, i64) {
    buckets
        .iter()
        .filter_map(|bucket| {
            bucket
                .iter()
                .min_by(|(score, _, d), (other_score, _, other_d)| {
                    score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
                })
        })
        .max_by(|(score, _, d), (other_score, _, other_d)| {
            score
                .partial_cmp(other_score)
                // Choose the move with the highest depth if the scores are equal
                .unwrap_or(Ordering::Equal)
        })
        .map(|(&score, &mv, &d)| (score, mv, d))
        .unwrap_or((f32::NEG_INFINITY, Move::Down, depth))
}

/// Currently caching costs us more than it saves since the eval function is so fast
#[cfg(feature = "caching")]
pub static EVAL_CACHE: once_cell::sync::Lazy<dashmap::DashMap<CellBoard4Snakes11x11, f32>> =
    once_cell::sync::Lazy::new(dashmap::DashMap::new);

pub fn evaluate_board(
    cellboard: CellBoard4Snakes11x11,
    you: &SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
) -> f32 {
    #[cfg(feature = "caching")]
    if let Some(cached) = EVAL_CACHE.get(&cellboard) {
        return *cached;
    }
    let res = evaluate_for_player(cellboard, you)
        - snake_ids
            .clone()
            .iter()
            .filter(|&id| id != you)
            .map(|id| evaluate_for_player(cellboard, id))
            .sum::<f32>()
            / 3.0;
    #[cfg(feature = "caching")]
    EVAL_CACHE.insert(cellboard, res);
    res
}

fn evaluate_for_player(cellboard: CellBoard4Snakes11x11, you: &SnakeId) -> f32 {
    cellboard.get_health(you) as f32 / 10.0
        + cellboard.get_length(you) as f32
        + cellboard
            .possible_moves(&cellboard.get_head_as_native_position(you))
            .count()
            .pow(2) as f32
}

#[derive(Debug)]
pub struct Simulator {}

impl SimulatorInstruments for Simulator {
    fn observe_simulation(&self, _d: Duration) {}
}

pub fn is_won<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    you: &SnakeId,
) -> bool {
    cellboard.get_winner() == Some(*you)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_state;
    use battlesnake_game_types::compact_representation::StandardCellBoard4Snakes11x11;
    use battlesnake_game_types::types::build_snake_id_map;
    use battlesnake_game_types::wire_representation::Game;
    use simd_json::prelude::ArrayTrait;
    use std::cmp::Ordering;
    use std::collections::HashMap;

    fn test_board() -> StandardCellBoard4Snakes11x11 {
        let board = r##"{"game":{"id":"7417b69a-bbe9-47f3-b88b-db0e7e33cd48","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":51,"board":{"height":11,"width":11,"snakes":[{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RpJkFVGrG6W68bhQMxp6G738","name":"Hungry Bot","latency":"1","health":97,"body":[{"x":7,"y":0},{"x":6,"y":0},{"x":5,"y":0},{"x":4,"y":0},{"x":4,"y":1},{"x":4,"y":2},{"x":3,"y":2},{"x":2,"y":2},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":3}],"head":{"x":7,"y":0},"length":11,"shout":"","squad":"","customizations":{"color":"#00cc00","head":"alligator","tail":"alligator"}}],"food":[{"x":7,"y":9}],"hazards":[]},"you":{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##;
        let mut hm = HashMap::new();
        hm.insert(
            "7417b69a-bbe9-47f3-b88b-db0e7e33cd48".to_string(),
            build_snake_id_map(&serde_json::from_str::<Game>(board).unwrap()),
        );
        let g = decode_state(board.to_string(), Arc::new(Mutex::new(hm)));
        g.unwrap()
    }

    #[test]
    fn test_calc_move() {
        let board = test_board();
        calc_move(board, 3, Instant::now());
    }

    #[test]
    fn dedup_vec() {
        let v = vec![
            (5.0, Move::Down),
            (10.0, Move::Down),
            (-5.0, Move::Down),
            (3.0, Move::Up),
            (-5.0, Move::Up),
            (10.0, Move::Left),
            (7.0, Move::Left),
        ];
        let mut buckets = vec![vec![]; 4];
        for (score, mv) in v.iter() {
            buckets[mv.as_index()].push((score, mv));
        }
        let max_move = buckets
            .iter()
            .filter_map(|bucket| {
                bucket.iter().min_by(|(score, _), (other_score, _)| {
                    score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
                })
            })
            .max_by(|(score, _), (other_score, _)| {
                score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
            })
            .unwrap();
        println!("{:?}", max_move);
    }

    #[test]
    fn test_simulate_moves() {
        let board = test_board();
        println!("{}", board);
        let simulations = board.simulate(&Simulator {}, board.get_snake_ids().to_vec());
        let simulation: Vec<(Action<4>, CellBoard4Snakes11x11)> = simulations.collect();
        assert_eq!(6, simulation.len());
    }

    #[test]
    fn test_eval() {
        // TODO
    }
}
