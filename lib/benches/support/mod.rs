#![allow(dead_code)]
use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        FoodGettableGame, HealthGettableGame, Move, SnakeId, VictorDeterminableGame,
        YouDeterminableGame, build_snake_id_map,
    },
    wire_representation::{Game, Position},
};
use lib::mcts::{Node, SearchDepthStats, bench::search_with_rng};
use rand::{SeedableRng, rngs::SmallRng};
use serde::Deserialize;
use std::sync::Arc;

pub const SEED: u64 = 0x0BA7_71E5;
pub const SEARCH_BATCH: u64 = 512;
pub const WARMUP_ITERATIONS: u64 = 2048;
pub const DECISION_ITERATIONS: u64 = 16_384;
#[derive(Deserialize)]
struct WireFixture {
    name: String,
    game: Game,
    action_snakes: Vec<String>,
    actions: Vec<Vec<[u8; 2]>>,
}
pub struct Fixture {
    pub name: String,
    pub board: CellBoard4Snakes11x11,
    pub you: SnakeId,
    pub food: ArrayVec<Position, 121>,
    pub actions: Vec<ArrayVec<(SnakeId, Move), 4>>,
}
pub fn fixtures() -> Vec<Fixture> {
    let cases: Vec<WireFixture> =
        serde_json::from_str(include_str!("../fixtures/arena.json")).expect("Arena fixtures");
    cases
        .into_iter()
        .map(|case| {
            let ids = build_snake_id_map(&case.game);
            let board: CellBoard4Snakes11x11 =
                case.game.as_cell_board(&ids).expect("compact Arena board");
            let you = *board.you_id();
            assert!(
                board.get_health(&you) > 0 && board.alive_snake_count() >= 2,
                "benchmark must contain a live decision"
            );
            let food = board.get_all_food_as_positions();
            let actions = case
                .actions
                .iter()
                .map(|action| {
                    action
                        .iter()
                        .map(|[original_id, mv]| {
                            let wire_id = &case.action_snakes[*original_id as usize];
                            (ids[wire_id], Move::from_index(*mv as usize))
                        })
                        .collect()
                })
                .collect();
            Fixture {
                name: case.name,
                board,
                you,
                food,
                actions,
            }
        })
        .collect()
}
pub fn rng() -> SmallRng {
    SmallRng::seed_from_u64(SEED)
}
pub struct Search {
    pub root: Arc<Node>,
    pub rng: SmallRng,
    pub stats: SearchDepthStats,
}
impl Search {
    pub fn new(f: &Fixture, warmup: u64) -> Self {
        let mut s = Self {
            root: Arc::new(Node::new_root(f.board)),
            rng: rng(),
            stats: SearchDepthStats::default(),
        };
        if warmup > 0 {
            s.run(f.you, warmup);
        }
        s
    }
    pub fn run(&mut self, you: SnakeId, iterations: u64) {
        search_with_rng(&self.root, &you, &mut self.stats, &mut self.rng, iterations);
    }
}
