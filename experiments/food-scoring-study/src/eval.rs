use std::cell::Cell;

use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::SnakeId;

thread_local! {
    static FOOD_SCORING: Cell<bool> = const { Cell::new(false) };
}

pub fn use_food_scoring(enabled: bool) {
    FOOD_SCORING.with(|variant| variant.set(enabled));
}

pub fn evaluate_board(board: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    if FOOD_SCORING.with(Cell::get) {
        lib::eval::evaluate_board(board, you)
    } else {
        lib::eval::evaluate_board_with_length_weight(board, you, 3)
    }
}
