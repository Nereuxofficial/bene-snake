use std::sync::Arc;

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use lib::mcts::{Node, SearchDepthStats};

fn main() {
    for (name, fixture) in [
        (
            "opening",
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
        ),
        (
            "late",
            include_str!("../../battlesnake-game-types/fixtures/late_stage.json"),
        ),
    ] {
        let game: Game = serde_json::from_str(fixture).unwrap();
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let you = *board.you_id();
        let root = Arc::new(Node::new_root_with_rollout_depth(board, 64));
        let mut stats = SearchDepthStats::default();
        for _ in 0..10_000 {
            root.rollout(&you, &mut stats);
        }
        println!(
            "{name}: cap hits={} / 10000, max depth={}",
            stats.rollout_depth_limit_hits, stats.max_rollout_depth
        );
    }
}
