use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::CellBoard;
use battlesnake_game_types::compact_representation::CellNum;
use battlesnake_game_types::types::{
    Action, FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
    ReasonableMovesGame, SimulableGame, SimulatorInstruments, SnakeIDGettableGame, SnakeId,
    VictorDeterminableGame, YouDeterminableGame,
};
use std::time::Duration;

pub fn calc_move<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
) -> Move {
    let reasonable_moves = cellboard.reasonable_moves_for_each_snake();
    let you = cellboard.you_id();
    search_paranoid_minimax(cellboard, 3, &you)
}

/// Given a cellboard, a depth and a SnakeId, this function will search for the best move
/// using a paranoid minimax algorithm.
/// **Inner Workings**
/// We collect all reasonable moves in our current state and simulate each one for our current position.
/// Then we simulate the next opponent moves and pick the least favorable outcome for us.
/// We then simulate the next opponent moves and pick the least favorable outcome for us.
/// Repeat until there are no more opponents left on the current board.
pub fn search_paranoid_minimax<
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    you: &SnakeId,
) -> Move {
    let reasonable_moves = cellboard.reasonable_moves_for_each_snake();
    let simulated_boards = cellboard.simulate_with_moves(&Simulator {}, reasonable_moves);
    simulated_boards
        .map(|(action, a)| (evaluate_board(a, you), (action, a)))
        .max_by(|(e, _), (other_e, _)| e.total_cmp(other_e))
        .map(|(_, (action, _))| action.own_move())
        .unwrap_or(Move::Down)
}

struct Node<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> {
    board: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    actions: Vec<Action<MAX_SNAKES>>,
    value: f32,
}

fn paranoid_min_value<
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    depth: usize,
    snake_ids: &[SnakeId],
) -> f32 {
    let mut value = f32::INFINITY;
    let mut current_board = cellboard;
    value
}

/// Given a state and a player id returns a list of boards with the next possible reasonable moves of that player.
fn next_boards<
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

    fn test_board() -> StandardCellBoard4Snakes11x11 {
        let mut board = r##"{"game":{"id":"7417b69a-bbe9-47f3-b88b-db0e7e33cd48","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":51,"board":{"height":11,"width":11,"snakes":[{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RpJkFVGrG6W68bhQMxp6G738","name":"Hungry Bot","latency":"1","health":97,"body":[{"x":7,"y":0},{"x":6,"y":0},{"x":5,"y":0},{"x":4,"y":0},{"x":4,"y":1},{"x":4,"y":2},{"x":3,"y":2},{"x":2,"y":2},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":3}],"head":{"x":7,"y":0},"length":11,"shout":"","squad":"","customizations":{"color":"#00cc00","head":"alligator","tail":"alligator"}}],"food":[{"x":7,"y":9}],"hazards":[]},"you":{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##;
        let g = decode_state(board.to_string());
        g.unwrap()
    }
}
