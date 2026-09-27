//! Reproducible probe for the tree-only head-to-head pruning change.
//!
//! This harness deliberately uses only `Node`'s public surface
//! (`new_root`, `best_move`, `search_once`) so the exact same file can be compiled
//! against the pre-change baseline and the changed crate for an apples-to-apples
//! comparison. `cargo bench`/`cargo run --release` both work.
//!
//! `cache_init` is fully deterministic for a fixed board: it times constructing a
//! node and forcing its move cache (`best_move` initializes the cache). That isolates
//! the only work this change adds, which is the per-node pruned move list.
//!
//! `search` runs a fixed number of `search_once` iterations on a fresh root per
//! sample. The search itself uses thread-local RNG, so those numbers are not bit-for-bit
//! deterministic; the probe reports the sample spread so noise is visible. Do not claim
//! a search throughput gain unless the changed and baseline spreads are disjoint.
//!
//! Usage:
//!   cargo bench --package lib --bench mcts_pruning
//! Optional environment overrides:
//!   MCTS_PROBE_CACHE_SAMPLES, MCTS_PROBE_CACHE_ITERS,
//!   MCTS_PROBE_SEARCH_SAMPLES, MCTS_PROBE_SEARCH_ITERS

use std::{
    env,
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{SnakeId, YouDeterminableGame, build_snake_id_map},
    wire_representation::{BattleSnake, Game, Position},
};
use lib::mcts::{Node, SearchDepthStats, search_once};

fn env_u64(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn board_from_specs(
    own_body: &[Position],
    own_health: i32,
    opponents: &[(&[Position], i32)],
    food: Vec<Position>,
) -> CellBoard4Snakes11x11 {
    let mut game: Game =
        serde_json::from_str(include_str!("../fixtures/turn33-food.json")).expect("valid fixture");
    let own_wire_id = game.you.id.clone();
    let template = game
        .board
        .snakes
        .iter()
        .find(|snake| snake.id != own_wire_id)
        .expect("fixture has an opponent")
        .clone();

    let set_body = |snake: &mut BattleSnake, body: &[Position], health: i32| {
        snake.head = body[0];
        snake.body = body.iter().copied().collect();
        snake.health = health;
    };

    set_body(&mut game.you, own_body, own_health);
    let mut snakes = vec![game.you.clone()];
    for (index, (body, health)) in opponents.iter().enumerate() {
        let mut snake = template.clone();
        snake.id = format!("gs_opponent_{index}");
        snake.name = format!("opponent-{index}");
        set_body(&mut snake, body, *health);
        snakes.push(snake);
    }
    game.board.snakes = snakes;
    game.board.food = food;

    let ids = build_snake_id_map(&game);
    game.as_cell_board(&ids).expect("valid board")
}

/// Far-apart heads: no candidate destination is contested, so pruning keeps everything.
fn uncontested_fixture() -> CellBoard4Snakes11x11 {
    board_from_specs(
        &[Position::new(5, 5), Position::new(5, 4)],
        100,
        &[(&[Position::new(2, 9), Position::new(2, 10)], 100)],
        Vec::new(),
    )
}

/// Equal-length heads two squares apart that both can enter (5, 6): the tree must drop
/// our Up move, so cache initialization exercises real pruning.
fn head_to_head_fixture() -> CellBoard4Snakes11x11 {
    board_from_specs(
        &[Position::new(5, 5), Position::new(5, 4)],
        100,
        &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
        Vec::new(),
    )
}

#[derive(Default)]
struct Depth {
    max_rollout_depth: u32,
    max_tree_depth: usize,
}

struct Stats {
    median: f64,
    min: f64,
    max: f64,
}

impl Stats {
    fn from_ns(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        let median = samples[samples.len() / 2];
        Self {
            median,
            min: samples[0],
            max: samples[samples.len() - 1],
        }
    }

    fn spread_percent(&self) -> f64 {
        if self.median == 0.0 {
            0.0
        } else {
            100.0 * (self.max - self.min) / self.median
        }
    }
}

fn time_cache_init(board: CellBoard4Snakes11x11, you: SnakeId, iters: u64, samples: u64) -> Stats {
    // Warm up so allocator/page-cache effects are not charged to the first sample.
    for _ in 0..(iters / 10).max(1) {
        let node = Node::new_root(black_box(board));
        black_box(node.best_move(black_box(you)));
    }

    let mut per_iter = Vec::with_capacity(samples as usize);
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..iters {
            let node = Node::new_root(black_box(board));
            black_box(node.best_move(black_box(you)));
        }
        per_iter.push(start.elapsed().as_nanos() as f64 / iters as f64);
    }
    Stats::from_ns(per_iter)
}

fn time_search(
    board: CellBoard4Snakes11x11,
    you: SnakeId,
    iters: u64,
    samples: u64,
) -> (Stats, Depth) {
    for _ in 0..(iters / 10).max(1) {
        let root = Arc::new(Node::new_root(black_box(board)));
        let mut stats = SearchDepthStats::default();
        search_once(&root, black_box(&you), &mut stats);
    }

    let mut per_iter = Vec::with_capacity(samples as usize);
    let mut depth = Depth::default();
    for _ in 0..samples {
        let root = Arc::new(Node::new_root(black_box(board)));
        let mut stats = SearchDepthStats::default();
        let start = Instant::now();
        for _ in 0..iters {
            search_once(&root, black_box(&you), &mut stats);
        }
        let elapsed = start.elapsed();
        black_box(root.best_move(you));
        depth.max_rollout_depth = depth.max_rollout_depth.max(stats.max_rollout_depth);
        depth.max_tree_depth = depth.max_tree_depth.max(stats.max_tree_depth);
        per_iter.push(elapsed.as_nanos() as f64 / iters as f64);
    }
    (Stats::from_ns(per_iter), depth)
}

fn report(
    name: &str,
    fixture: &str,
    stats: &Stats,
    iters: u64,
    samples: u64,
    depth: Option<&Depth>,
) {
    let depth = depth.map_or_else(
        || String::from("max_rollout_depth=na max_tree_depth=na"),
        |depth| {
            format!(
                "max_rollout_depth={} max_tree_depth={}",
                depth.max_rollout_depth, depth.max_tree_depth
            )
        },
    );
    println!(
        "mcts_pruning fixture={fixture} probe={name} ns_per_iter_median={:.1} \
         ns_per_iter_min={:.1} ns_per_iter_max={:.1} spread_pct={:.1} \
         iters_per_sample={iters} samples={samples} {depth}",
        stats.median,
        stats.min,
        stats.max,
        stats.spread_percent()
    );
}

fn main() {
    let cache_samples = env_u64("MCTS_PROBE_CACHE_SAMPLES", 15);
    let cache_iters = env_u64("MCTS_PROBE_CACHE_ITERS", 20_000);
    let search_samples = env_u64("MCTS_PROBE_SEARCH_SAMPLES", 31);
    let search_iters = env_u64("MCTS_PROBE_SEARCH_ITERS", 256);

    println!(
        "mcts_pruning build=release cache_samples={cache_samples} cache_iters={cache_iters} \
         search_samples={search_samples} search_iters={search_iters}"
    );

    for (fixture, board) in [
        ("uncontested", uncontested_fixture()),
        ("head_to_head", head_to_head_fixture()),
    ] {
        let you = *board.you_id();

        let cache = time_cache_init(board, you, cache_iters, cache_samples);
        report(
            "cache_init",
            fixture,
            &cache,
            cache_iters,
            cache_samples,
            None,
        );

        let (search, depth) = time_search(board, you, search_iters, search_samples);
        report(
            "search",
            fixture,
            &search,
            search_iters,
            search_samples,
            Some(&depth),
        );
    }

    // Keep the process alive long enough for any straggling stdout flush; also documents
    // that the probe intentionally does no cleanup work.
    std::thread::sleep(Duration::from_millis(1));
}
