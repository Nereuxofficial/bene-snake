#![feature(portable_simd)]

mod mcts;
mod tree;

use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::{CellBoard, CellBoard4Snakes11x11};
use battlesnake_game_types::compact_representation::CellNum;
use battlesnake_game_types::types::{
    Action, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
    NeighborDeterminableGame, ReasonableMovesGame, SimulableGame, SimulatorInstruments,
    SnakeIDGettableGame, SnakeIDMap, SnakeId, VictorDeterminableGame, YouDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use rayon::prelude::*;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use tracing::info;
use tracing::instrument;

pub const DELAY: Duration = Duration::from_millis(150);

pub type GameStates = Arc<Mutex<HashMap<String, SnakeIDMap>>>;

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

pub fn calc_move(cellboard: CellBoard4Snakes11x11, depth: i64, start: Instant) -> Move {
    let you = cellboard.you_id();
    let snake_ids = cellboard.get_snake_ids();
    let instant_ref = &start;
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
fn paranoid_minimax(
    game: CellBoard4Snakes11x11,
    depth: i64,
    you: &SnakeId,
    snake_ids: Cow<[SnakeId]>,
    start: &Instant,
) -> (u16, Move, i64) {
    if is_won(game, you) {
        return (u16::MAX, Move::Down, depth);
    }
    if !game.is_alive(you) {
        return (0, Move::Down, depth);
    }
    #[cfg(feature = "bench")]
    if depth == 0 {
        return (evaluate_board(&game, you, snake_ids), Move::Down, depth);
    }
    #[cfg(not(feature = "bench"))]
    if start.elapsed() + DELAY > Duration::from_millis(500) || depth == 0 {
        return (evaluate_board(&game, you, snake_ids), Move::Down, depth);
    }
    let mut moves = game.reasonable_moves_for_each_snake();
    let simulations: Vec<(Action<4>, CellBoard4Snakes11x11)> = game
        .simulate_with_moves(&Simulator {}, &mut moves)
        .collect();
    let recursive_scores: Vec<(u16, Move, i64)> = simulations
        .par_iter()
        .map(|(action, b)| {
            let res = paranoid_minimax(*b, depth - 1, you, snake_ids.clone(), start);
            (res.0, action.own_move(), res.2)
        })
        .collect();
    let mut buckets = vec![vec![]; 4];
    for (score, mv, depth) in recursive_scores {
        buckets[mv.as_index()].push((score, mv, depth));
    }
    get_best_move_from_buckets(buckets.as_slice())
}

fn get_best_move_from_buckets(buckets: &[Vec<(u16, Move, i64)>]) -> (u16, Move, i64) {
    buckets
        .iter()
        .filter_map(|bucket| {
            bucket.iter().min_by(|(score, _, _), (other_score, _, _)| {
                score.partial_cmp(other_score).unwrap_or(Ordering::Equal)
            })
        })
        .max_by(|(score, _, _), (other_score, _, _)| {
            score
                .partial_cmp(other_score)
                // Choose the move with the highest depth if the scores are equal
                .unwrap_or(Ordering::Equal)
        })
        .map(|(score, mv, d)| (*score, *mv, *d))
        .unwrap()
}

pub fn evaluate_board(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    snake_ids: Cow<[SnakeId]>,
) -> u16 {
    let res = evaluate_for_player(cellboard, you)
        - snake_ids
            .iter()
            .filter(|&id| id != you)
            .map(|id| evaluate_for_player(cellboard, id))
            .sum::<u16>()
            / snake_ids.len() as u16;
    res
}

fn evaluate_for_player(cellboard: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    cellboard.get_health(you) as u16
        + cellboard.get_length(you) / 10
        + cellboard
            .possible_moves(&cellboard.get_head_as_native_position(you))
            .count() as u16
            * 2
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
    use battlesnake_game_types::types::{build_snake_id_map, ReasonableMovesGame};
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
        let simulations = board.simulate(&Simulator {}, &board.get_snake_ids());
        let simulation: Vec<(Action<4>, CellBoard4Snakes11x11)> = simulations.collect();
        assert_eq!(6, simulation.len());
    }

    #[test]
    fn test_available_threads() {
        // sorry this test is pretty manual :c
        let threads = std::thread::available_parallelism().unwrap().get();
        println!("Threads: {}", threads);
    }
    #[test]
    fn test_possible_moves() {
        let board = test_board();
        let head = board.get_head_as_native_position(board.you_id());
        let moves: Vec<Move> = board.possible_moves(&head).map(|(m, _)| m).collect();
        let reasonable_moves = board
            .reasonable_moves_for_each_snake()
            .filter_map(|(id, moves)| {
                if id == *board.you_id() {
                    Some(moves)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_ne!(moves, *reasonable_moves.first().unwrap());
    }

    #[test]
    fn does_not_do_dumb_move() {
        let board_string = r##"{"game":{"id":"038fb980-3e76-4c06-b60e-07251422175f","ruleset":{"name":"standard","version":"?","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0}},"map":"standard","timeout":500,"source":"custom"},"turn":255,"board":{"width":11,"height":11,"food":[{"x":4,"y":0},{"x":3,"y":4}],"hazards":[],"snakes":[{"id":"gs_Cx6Rxgx8qbKCdKHvK4bwWTfS","name":"Kakemonsteret-v2","health":91,"body":[{"x":10,"y":9},{"x":10,"y":10},{"x":9,"y":10},{"x":9,"y":9},{"x":9,"y":8},{"x":8,"y":8},{"x":8,"y":7},{"x":8,"y":6},{"x":8,"y":5},{"x":7,"y":5},{"x":7,"y":6},{"x":6,"y":6},{"x":6,"y":5},{"x":5,"y":5},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":4,"y":6},{"x":4,"y":7}],"latency":48,"head":{"x":10,"y":9},"length":19,"shout":"","squad":"","customizations":{"color":"#9400d3","head":"default","tail":"default"}},{"id":"gs_dQ9PSkYTCfKffD9WyJ39QB66","name":"bene-snake","health":100,"body":[{"x":3,"y":6},{"x":2,"y":6},{"x":2,"y":5},{"x":1,"y":5},{"x":1,"y":4},{"x":2,"y":4},{"x":2,"y":3},{"x":2,"y":2},{"x":2,"y":2}],"latency":470,"head":{"x":3,"y":6},"length":9,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}},{"id":"gs_Gv9SPCBHmJ8bWvp6D9wJj3fc","name":"Geriatric Jagwire","health":76,"body":[{"x":5,"y":2},{"x":4,"y":2},{"x":4,"y":1},{"x":3,"y":1},{"x":3,"y":2},{"x":3,"y":3},{"x":4,"y":3},{"x":5,"y":3},{"x":6,"y":3},{"x":6,"y":4},{"x":7,"y":4},{"x":7,"y":3},{"x":8,"y":3}],"latency":24,"head":{"x":5,"y":2},"length":13,"shout":"","squad":"","customizations":{"color":"#ffe58f","head":"glasses","tail":"freckled"}}]},"you":{"id":"gs_dQ9PSkYTCfKffD9WyJ39QB66","name":"bene-snake","health":100,"body":[{"x":3,"y":6},{"x":2,"y":6},{"x":2,"y":5},{"x":1,"y":5},{"x":1,"y":4},{"x":2,"y":4},{"x":2,"y":3},{"x":2,"y":2},{"x":2,"y":2}],"latency":470,"head":{"x":3,"y":6},"length":9,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}}"##;
        let board = serde_json::from_str::<Game>(board_string).unwrap();
        let cb = board.as_cell_board(&build_snake_id_map(&board)).unwrap();
        let r#move = calc_move(cb, 3, Instant::now());
        assert_ne!(r#move, Move::Down);
    }
}
