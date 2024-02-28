use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::{CellBoard, CellBoard4Snakes11x11};
use battlesnake_game_types::compact_representation::CellNum;
use battlesnake_game_types::types::{
    Action, HealthGettableGame, LengthGettableGame, Move, SimulableGame, SimulatorInstruments,
    SnakeIDGettableGame, SnakeIDMap, SnakeId, VictorDeterminableGame, YouDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use std::borrow::Cow;
use std::cmp::{max, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use tracing::info;
#[cfg(debug_assertions)]
use tracing::info;

pub const DELAY: Duration = Duration::from_millis(150);

pub type GameStates = Arc<Mutex<HashMap<String, SnakeIDMap>>>;

pub fn decode_state(
    mut text: String,
    game_states: GameStates,
) -> color_eyre::Result<CellBoard4Snakes11x11> {
    #[cfg(debug_assertions)]
    info!("JSON: {}", text);
    let decoded: Game = unsafe { simd_json::serde::from_str(&mut text) }?;
    let cellboard = decoded
        .as_cell_board(
            game_states
                .lock()
                .unwrap()
                .get(&decoded.game.id)
                .expect("No such game id found"),
        )
        .unwrap();
    Ok(cellboard)
}

pub fn calc_move<
    T: CellNum + Send,
    D: Dimensions + Send + 'static,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    start: Instant,
) -> Move {
    let you = cellboard.you_id();
    let snake_ids = cellboard.get_snake_ids();
    let instant_ref = Arc::new(start);
    paranoid_minimax(cellboard, depth, *you, Cow::Owned(snake_ids), instant_ref).1
}

/// Given a cellboard, a depth and a SnakeId, this function will search for the best move
/// using a paranoid minimax algorithm.
/// **Inner Workings**
/// We collect all reasonable moves in our current state and simulate each one for our current position.
/// Then we simulate the next opponent moves and pick the least favorable outcome for us.
/// We then simulate the next opponent moves and pick the least favorable outcome for us.
/// Repeat until there are no more opponents left on the current board.
/// In Paranoid Minimax, we want to maximize our own score and minimize the score of our opponents.
fn paranoid_minimax<
    T: CellNum + Send + 'static,
    D: Dimensions + Send + 'static,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    game: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    you: SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
    start: Arc<Instant>,
) -> (f32, Move) {
    if is_won(game, &you) {
        return (f32::INFINITY, Move::Down);
    }
    if !game.is_alive(&you) {
        return (f32::NEG_INFINITY, Move::Down);
    }
    #[cfg(feature = "bench")]
    if depth == 0 {
        return (evaluate_board(game, &you, snake_ids), Move::Down);
    }
    #[cfg(not(feature = "bench"))]
    if start.elapsed() + DELAY > Duration::from_millis(500) {
        return (evaluate_board(game, &you, snake_ids), Move::Down);
    }
    let simulations = game.simulate(&Simulator {}, game.get_snake_ids().to_vec());
    let recursive_scores: Vec<(f32, Move)> = {
        let simulation: Vec<(Action<MAX_SNAKES>, CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>)> =
            simulations.collect();
        let count = simulation.len();
        let mut scores = Vec::with_capacity(count);
        let threads = rayon::current_num_threads();
        let mut tasks = Vec::with_capacity(threads);
        // Distribute the work across the threads
        // Split simulations into threads chunks
        for chunk in simulation.chunks(max(count / threads, 1)) {
            let chunk = chunk.to_vec();
            let snake_ids_clone = snake_ids.to_vec();
            let start_clone = start.clone();
            tasks.push(std::thread::spawn(move || {
                chunk
                    .into_iter()
                    .map(|(action, b)| {
                        (
                            paranoid_minimax_single_threaded(
                                b,
                                depth - 1,
                                you,
                                Cow::Owned(snake_ids_clone.clone()),
                                start_clone.clone(),
                            )
                            .0,
                            action.own_move(),
                        )
                    })
                    .collect::<Vec<(f32, Move)>>()
            }));
        }
        // Collect the results
        for task in tasks {
            scores.append(&mut task.join().unwrap());
        }
        scores
    };

    let mut buckets = vec![vec![]; 4];
    for (score, mv) in recursive_scores.iter() {
        buckets[mv.as_index()].push((score, mv));
    }
    buckets
        .iter()
        .filter_map(|bucket| {
            bucket.iter().min_by(|(score, _), (other_score, _)| {
                score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
            })
        })
        .max_by(|(score, _), (other_score, _)| {
            score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
        })
        .map(|(&score, &mv)| (score, mv))
        .unwrap_or((f32::NEG_INFINITY, Move::Down))
}

pub fn paranoid_minimax_single_threaded<
    T: CellNum + Send + 'static,
    D: Dimensions + Send + 'static,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    game: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    you: SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
    start: Arc<Instant>,
) -> (f32, Move) {
    if is_won(game, &you) {
        return (f32::INFINITY, Move::Down);
    }
    if !game.is_alive(&you) {
        return (f32::NEG_INFINITY, Move::Down);
    }
    #[cfg(feature = "bench")]
    if depth == 0 {
        return (evaluate_board(game, &you, snake_ids), Move::Down);
    }
    #[cfg(not(feature = "bench"))]
    if start.elapsed() + DELAY > Duration::from_millis(500) {
        return (evaluate_board(game, &you, snake_ids), Move::Down);
    }
    let simulations = game.simulate(&Simulator {}, game.get_snake_ids().to_vec());
    let recursive_scores: Vec<(f32, Move)> = simulations
        .map(|(action, b)| {
            (
                #[cfg(feature = "bench")]
                paranoid_minimax(b, depth - 1, you, snake_ids.clone(), start.clone()).0,
                #[cfg(not(feature = "bench"))]
                paranoid_minimax(b, depth, you, snake_ids.clone(), start.clone()).0,
                action.own_move(),
            )
        })
        .collect();
    let mut buckets = [vec![], vec![], vec![], vec![]];
    for (score, mv) in recursive_scores.iter() {
        buckets[mv.as_index()].push((score, mv));
    }
    buckets
        .iter()
        .filter_map(|bucket| {
            bucket.iter().min_by(|(score, _), (other_score, _)| {
                score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
            })
        })
        .max_by(|(score, _), (other_score, _)| {
            score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
        })
        .map(|(&score, &mv)| (score, mv))
        .unwrap_or((f32::NEG_INFINITY, Move::Down))
}

pub fn evaluate_board<
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    you: &SnakeId,
    snake_ids: Cow<Vec<SnakeId>>,
) -> f32 {
    evaluate_for_player(cellboard, you)
        - snake_ids
            .clone()
            .iter()
            .filter(|&id| id != you)
            .map(|id| evaluate_for_player(cellboard, id))
            .sum::<f32>()
            / 3.0
}

fn evaluate_for_player<
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    you: &SnakeId,
) -> f32 {
    cellboard.get_health(you) as f32 / 10.0 + cellboard.get_length(you) as f32
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
    use std::sync::Arc;

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
}
