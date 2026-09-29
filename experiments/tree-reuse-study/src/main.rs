use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{Move, SnakeId},
};
use gym::runner::GameConfig;
use lib::{
    Agent,
    mcts::{Node, SearchTreeCache, mcts_search},
};

#[derive(Default, Clone, Copy)]
struct Metrics {
    calls: u64,
    reuse_hits: u64,
    carried_visits: u64,
    new_iterations: u64,
    retained_candidates: u64,
    search_wall_us: u64,
    total_wall_us: u64,
    cache_lookup_us: u64,
    cache_prepare_us: u64,
}

struct Variant {
    reuse: bool,
    think_time: Duration,
    cache: Mutex<Option<SearchTreeCache>>,
    metrics: Mutex<Metrics>,
}

impl Variant {
    fn new(reuse: bool, think_time: Duration) -> Self {
        Self {
            reuse,
            think_time,
            cache: Mutex::new(None),
            metrics: Mutex::new(Metrics::default()),
        }
    }

    fn reset(&self) {
        *self.cache.lock().unwrap() = None;
        *self.metrics.lock().unwrap() = Metrics::default();
    }

    fn metrics(&self) -> Metrics {
        *self.metrics.lock().unwrap()
    }
}

impl Agent for Variant {
    fn name(&self) -> &str {
        if self.reuse { "reuse" } else { "fresh" }
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let call_start = Instant::now();
        let lookup_start = Instant::now();
        let reused = if self.reuse {
            self.cache
                .lock()
                .unwrap()
                .take()
                .and_then(|cache| cache.match_observed(board, you))
        } else {
            None
        };
        let cache_lookup_us = lookup_start.elapsed().as_micros() as u64;
        let reuse_hit = reused.is_some();
        let root = reused.unwrap_or_else(|| Arc::new(Node::new_root(*board)));
        let carried_visits = root.visits();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_root = Arc::clone(&root);
        let worker_stop = Arc::clone(&stop);
        let search_start = Instant::now();
        let search = thread::spawn(move || mcts_search(worker_root, &you, worker_stop));
        thread::sleep(self.think_time);
        stop.store(true, Ordering::Relaxed);
        search.join().unwrap();
        let elapsed = search_start.elapsed();
        let chosen = root.best_move(you).unwrap_or(Move::Up);
        let prepare_start = Instant::now();
        let retained_candidates = if self.reuse {
            let cache = SearchTreeCache::after_move(&root, you, chosen);
            let count = cache.candidate_count();
            *self.cache.lock().unwrap() = Some(cache);
            count
        } else {
            0
        };
        let cache_prepare_us = prepare_start.elapsed().as_micros() as u64;
        let mut metrics = self.metrics.lock().unwrap();
        metrics.calls += 1;
        metrics.reuse_hits += u64::from(reuse_hit);
        metrics.carried_visits += u64::from(carried_visits);
        metrics.new_iterations += u64::from(root.visits() - carried_visits);
        metrics.retained_candidates += retained_candidates as u64;
        metrics.search_wall_us += elapsed.as_micros() as u64;
        metrics.cache_lookup_us += cache_lookup_us;
        metrics.cache_prepare_us += cache_prepare_us;
        metrics.total_wall_us += call_start.elapsed().as_micros() as u64;
        chosen
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 5,
        "usage: tree-reuse-study SEED MS MAX_TURNS RUN_ID"
    );
    let seed: u64 = args[1].parse().unwrap();
    let budget_ms: u64 = args[2].parse().unwrap();
    let max_turns: u32 = args[3].parse().unwrap();
    let run_id = &args[4];
    assert!(budget_ms > 0);

    let reused = Variant::new(true, Duration::from_millis(budget_ms));
    let fresh = Variant::new(false, Duration::from_millis(budget_ms));
    let config = GameConfig {
        max_turns,
        ..GameConfig::duel()
    };

    for reuse_seat in [0, 1] {
        reused.reset();
        fresh.reset();
        let agents: [&dyn Agent; 2] = if reuse_seat == 0 {
            [&reused, &fresh]
        } else {
            [&fresh, &reused]
        };
        let result = gym::runner::run_game_seeded(&agents, &config, seed);
        let outcome = result
            .winner
            .map(|winner| if winner == reuse_seat { "win" } else { "loss" })
            .unwrap_or("draw");
        let r = reused.metrics();
        let f = fresh.metrics();
        println!(
            "{}",
            serde_json::json!({
                "run_id": run_id,
                "seed": seed,
                "reuse_seat": reuse_seat,
                "budget_ms": budget_ms,
                "max_turns": max_turns,
                "turns": result.turns,
                "reuse_outcome": outcome,
                "reuse_calls": r.calls,
                "reuse_hits": r.reuse_hits,
                "reuse_carried_visits": r.carried_visits,
                "reuse_new_iterations": r.new_iterations,
                "reuse_retained_candidates": r.retained_candidates,
                "reuse_search_wall_us": r.search_wall_us,
                "reuse_total_wall_us": r.total_wall_us,
                "reuse_cache_lookup_us": r.cache_lookup_us,
                "reuse_cache_prepare_us": r.cache_prepare_us,
                "fresh_calls": f.calls,
                "fresh_new_iterations": f.new_iterations,
                "fresh_search_wall_us": f.search_wall_us,
                "fresh_total_wall_us": f.total_wall_us,
            })
        );
    }
}
