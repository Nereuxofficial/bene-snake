//! This is an NNUE implementation for battlesnake.

use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;

/// Our Features are the input to the neural network. They have a constant size and a single one is
/// generated from this: (Position, is_head, player_number)
/// On an 11x11 board for 4 Snakes this results in 11x11 x 2 x 4 = 968 features
struct Features([f32; 968]);

impl Into<Features> for CellBoard4Snakes11x11 {
    fn into(self) -> Features {
        let mut features = [0.0; 968];
        for (i, cell) in self.cells.iter().enumerate() {
            let x = i % 11;
            let y = i / 11;
            for (j, snake) in cell.snakes.iter().enumerate() {
                features[i * 8 + j * 2] = if snake.is_head { 1.0 } else { 0.0 };
                features[i * 8 + j * 2 + 1] = if snake.player_number == 0 { 1.0 } else { 0.0 };
            }
        }
        Features(features)
    }
}
