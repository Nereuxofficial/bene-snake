use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::SnakeId;

pub fn evaluate_board(board: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    lib::eval::evaluate_board(board, you)
}
