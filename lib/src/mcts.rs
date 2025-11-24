use std::borrow::Cow;

use battlesnake_game_types::types::{ReasonableMovesGame, SimulatorInstruments, SnakeId};

use crate::CellBoard4Snakes11x11;

struct Node {
    board: CellBoard4Snakes11x11,
}
#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}
impl Node {
    pub fn explore_fixed(&self, snake_ids: Cow<[SnakeId]>) {
        let mut moves = self.board.reasonable_moves_for_each_snake();
        //self.board.simulate_with_moves(&Instr, &mut mv);
    }
}
