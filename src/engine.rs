use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::CellBoard;
use battlesnake_game_types::compact_representation::{CellNum, StandardCellBoard4Snakes11x11};
use battlesnake_game_types::types::{
    FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
    ReasonableMovesGame, SimulableGame, SimulatorInstruments, SizeDeterminableGame,
    SnakeIDGettableGame, SnakeId, VictorDeterminableGame, YouDeterminableGame,
};
use std::time::Duration;
use tracing::info;

pub fn calc_move<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
) -> Move {
    let mut reasonable_moves = cellboard.reasonable_moves_for_each_snake();
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
    let mut reasonable_moves = cellboard.reasonable_moves_for_each_snake();
    let mut best_move = Move::Down;
    let simulated_boards = cellboard.simulate_with_moves(&Simulator {}, reasonable_moves);
    simulated_boards
        .map(|(action, a)| (evaluate_board(a, you), (action, a)))
        .max_by(|(e, _), (other_e, _)| e.total_cmp(other_e))
        .map(|(_, (action, _))| action.own_move())
        .unwrap_or(Move::Down)
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
            .unwrap_or(0) as f32;
        #[cfg(debug_assertions)]
        info!("Food distance avg: {}", food_distance_avg);
        -food_distance_avg
            + cellboard.get_health(you) as f32 / 10.0
            + cellboard.get_length(you) as f32 / 10.0
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
    fn observe_simulation(&self, d: Duration) {
        info!("Simulation took {:?}", d);
    }
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
