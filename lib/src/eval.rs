use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        HeadGettableGame, HealthGettableGame, LengthGettableGame, NeighborDeterminableGame, SnakeId,
    },
};

pub fn evaluate_board(cellboard: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    cellboard.get_health(you) as u16 / 10
        + cellboard.get_length(you)
        + cellboard
            .possible_moves(&cellboard.get_head_as_native_position(you))
            .count() as u16
            * 2
}
