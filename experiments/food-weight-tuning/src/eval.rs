use std::cell::Cell;

use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::SnakeId;

thread_local! {
    static FOOD_WEIGHT: Cell<i32> = const { Cell::new(12) };
    static LENGTH_WEIGHT: Cell<i32> = const { Cell::new(3) };
}

pub fn set_weights(food_weight: i32, length_weight: i32) {
    FOOD_WEIGHT.with(|slot| slot.set(food_weight));
    LENGTH_WEIGHT.with(|slot| slot.set(length_weight));
}

pub fn evaluate_board(board: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    FOOD_WEIGHT.with(|food| {
        LENGTH_WEIGHT.with(|length| {
            lib::eval::evaluate_board_with_food_weight(board, you, length.get(), food.get())
        })
    })
}
