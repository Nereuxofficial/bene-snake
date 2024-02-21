use crate::get_move;
use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::CellBoard;
use battlesnake_game_types::compact_representation::CellNum;
use battlesnake_game_types::types::{
    Action, FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
    ReasonableMovesGame, SimulableGame, SimulatorInstruments, SnakeIDGettableGame, SnakeId,
    VictorDeterminableGame, YouDeterminableGame,
};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::Duration;

pub fn calc_move<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
) -> Move {
    let reasonable_moves = cellboard.reasonable_moves_for_each_snake();
    let you = cellboard.you_id();
    paranoid_minimax(cellboard, 3, you, true).1
}

/// Given a cellboard, a depth and a SnakeId, this function will search for the best move
/// using a paranoid minimax algorithm.
/// **Inner Workings**
/// We collect all reasonable moves in our current state and simulate each one for our current position.
/// Then we simulate the next opponent moves and pick the least favorable outcome for us.
/// We then simulate the next opponent moves and pick the least favorable outcome for us.
/// Repeat until there are no more opponents left on the current board.
/// In Paranoid Minimax, we want to maximize our own score and minimize the score of our opponents.
fn paranoid_minimax<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    game: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    you: &SnakeId,
    top_level: bool,
) -> (f32, Move) {
    if is_lost(game, you) {
        return (f32::NEG_INFINITY, Move::Down);
    }
    if is_won(game, you) {
        return (f32::INFINITY, Move::Down);
    }
    if depth == 0 {
        return (evaluate_board(game, you), Move::Down);
    }
    let mut simulations = game.simulate(&Simulator {}, game.get_snake_ids());

    let recursive_scores: Vec<(f32, Move)> = simulations
        .map(|(action, b)| {
            (
                paranoid_minimax(b, depth - 1, you, false).0,
                action.own_move(),
            )
        })
        .collect();

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

/// Given a state and a player id returns a list of boards with the next possible reasonable moves of that player.
fn next_boards_for_player<
    T: CellNum,
    D: Dimensions + 'static,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    player: &SnakeId,
) -> Vec<(Action<MAX_SNAKES>, CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>)> {
    let mut reasonable_moves = cellboard
        .reasonable_moves_for_each_snake()
        .filter(|(id, _)| id == player);
    cellboard
        .simulate_with_moves(&Simulator {}, reasonable_moves)
        .collect()
}

fn evaluate_board<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    you: &SnakeId,
) -> f32 {
    evaluate_for_player(cellboard, you)
        - cellboard
            .get_snake_ids()
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
    if cellboard.is_alive(you) {
        let other_ids = cellboard.get_snake_ids();
        let food = cellboard.get_all_food_as_positions();
        let head = cellboard.get_head_as_position(you);
        // Favor positions closer to food
        let food_distance_avg = food
            .iter()
            .map(|&f| head.sub_vec(f.to_vector()).manhattan_length())
            .sum::<u32>()
            .checked_div(food.len() as u32)
            .unwrap_or(0) as f32
            / 3.0;
        -food_distance_avg
            + cellboard.get_health(you) as f32 / 10.0
            + cellboard.get_length(you) as f32
            - other_ids
                .iter()
                .filter(|&id| id != you)
                .map(|id| {
                    cellboard.get_health(id) as f32 / 100.0 + cellboard.get_length(id) as f32 / 10.0
                })
                .sum::<f32>()
    } else {
        f32::MIN
    }
}

#[derive(Debug)]
struct Simulator {}

impl SimulatorInstruments for Simulator {
    fn observe_simulation(&self, _d: Duration) {}
}

pub fn is_lost<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    you: &SnakeId,
) -> bool {
    !cellboard.is_alive(you)
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
    use std::cmp::{min, Ordering};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn test_board() -> StandardCellBoard4Snakes11x11 {
        let mut board = r##"{"game":{"id":"7417b69a-bbe9-47f3-b88b-db0e7e33cd48","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":51,"board":{"height":11,"width":11,"snakes":[{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RpJkFVGrG6W68bhQMxp6G738","name":"Hungry Bot","latency":"1","health":97,"body":[{"x":7,"y":0},{"x":6,"y":0},{"x":5,"y":0},{"x":4,"y":0},{"x":4,"y":1},{"x":4,"y":2},{"x":3,"y":2},{"x":2,"y":2},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":3}],"head":{"x":7,"y":0},"length":11,"shout":"","squad":"","customizations":{"color":"#00cc00","head":"alligator","tail":"alligator"}}],"food":[{"x":7,"y":9}],"hazards":[]},"you":{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##;
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
        let mv = calc_move(board);
        println!("{:?}", mv);
    }

    #[test]
    fn dedup_vec() {
        let mut v = vec![
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
