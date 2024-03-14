//! This is an NNUE implementation for battlesnake.

use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;

/// Our Features are the input to the neural network. They have a constant size and a single one is
/// generated from this: (Position, is_head, player_number)
/// On an 11x11 board for 4 Snakes this results in 11x11 x 2 x 4 = 968 features
struct Features([f32; 968]);

impl Features {}
