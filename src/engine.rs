use battlesnake_game_types::compact_representation::dimensions::Dimensions;
use battlesnake_game_types::compact_representation::standard::CellBoard;
use battlesnake_game_types::compact_representation::{
    CellIndex, CellNum, StandardCellBoard4Snakes11x11,
};
use battlesnake_game_types::types::{
    HeadGettableGame, HealthGettableGame, Move, NeighborDeterminableGame, ReasonableMovesGame,
    SizeDeterminableGame, SnakeId, VictorDeterminableGame, YouDeterminableGame,
};
use tracing::info;

pub fn calc_move<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>(
    cellboard: CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
) -> Move {
    let mut reasonable_moves = cellboard.reasonable_moves_for_each_snake();
    let you = cellboard.you_id();

    reasonable_moves
        .find(|&(snakeid, _)| snakeid == *you)
        .map(|(_, m)| *m.first().unwrap_or(&Move::Down))
        .unwrap_or(Move::Down)
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
