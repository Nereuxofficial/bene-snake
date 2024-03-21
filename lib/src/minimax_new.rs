use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{Move, ReasonableMovesGame};
use minimax::Winner;
// TODO: Modify the minimax crate to allow for more players
pub struct Game {
    to_move: u8,
}

impl minimax::Game for Game {
    type S = CellBoard4Snakes11x11;
    type M = Move;

    fn generate_moves(state: &Self::S, moves: &mut Vec<Self::M>) {
        let moves = state.reasonable_moves_for_each_snake();
    }

    fn apply(state: &mut Self::S, m: Self::M) -> Option<Self::S> {
        todo!()
    }

    fn get_winner(state: &Self::S) -> Option<Winner> {
        todo!()
    }
}
