#![allow(dead_code)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{FoodGettableGame, HeadGettableGame, LengthGettableGame, Move, SnakeId},
};
use gym::runner::GameConfig;
use lib::Agent;

mod eval;
#[path = "../../../lib/src/mcts.rs"]
mod mcts;

struct Variant {
    food_guidance: bool,
    food_scoring: bool,
    think_time: Duration,
    metrics: Mutex<Metrics>,
}

#[derive(Default)]
struct Metrics {
    last_length: Option<u16>,
    moves: u32,
    food_eaten: u32,
    moves_onto_food: u32,
    moves_closer_to_food: u32,
}

impl Variant {
    fn reset_metrics(&self) {
        *self.metrics.lock().unwrap() = Metrics::default();
    }
}

impl Agent for Variant {
    fn name(&self) -> &str {
        if self.food_guidance {
            "food-guided"
        } else {
            "baseline"
        }
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let head = board.get_head_as_position(&you);
        let current_length = board.get_length(&you);
        let food = board.get_all_food_as_positions();
        let nearest_food_distance = food
            .iter()
            .map(|pos| (head.x - pos.x).abs() + (head.y - pos.y).abs())
            .min();
        {
            let mut metrics = self.metrics.lock().unwrap();
            if let Some(last_length) = metrics.last_length {
                metrics.food_eaten += u32::from(current_length.saturating_sub(last_length));
            }
            metrics.last_length = Some(current_length);
        }
        let root = Arc::new(mcts::Node::new_root_with_food_guidance(
            *board,
            self.food_guidance,
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let search_root = Arc::clone(&root);
        let search_stop = Arc::clone(&stop);
        let food_scoring = self.food_scoring;
        let search = thread::spawn(move || {
            eval::use_food_scoring(food_scoring);
            mcts::mcts_search(search_root, &you, search_stop);
        });

        thread::sleep(self.think_time);
        stop.store(true, Ordering::Relaxed);
        let _ = search.join();
        let chosen = root.best_move(you).unwrap_or(Move::Up);
        let destination = head.add_vec(chosen.to_vector());
        let moves_onto_food = food.contains(&destination);
        let moves_closer_to_food = nearest_food_distance.is_some_and(|distance| {
            food.iter()
                .map(|pos| (destination.x - pos.x).abs() + (destination.y - pos.y).abs())
                .min()
                .is_some_and(|next| next < distance)
        });
        let mut metrics = self.metrics.lock().unwrap();
        metrics.moves += 1;
        metrics.moves_onto_food += u32::from(moves_onto_food);
        metrics.moves_closer_to_food += u32::from(moves_closer_to_food);
        chosen
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 6,
        "usage: food-guidance-study SEED_COUNT MS FIRST_SEED PAIR_ID MAX_TURNS"
    );
    let seeds: u64 = args[1].parse().unwrap();
    let think_time_ms: u64 = args[2].parse().unwrap();
    let first_seed: u64 = args[3].parse().unwrap();
    let pair_id = &args[4];
    let max_turns: u32 = args[5].parse().unwrap();
    assert!(seeds > 0 && think_time_ms > 0);

    let baseline = Variant {
        food_guidance: false,
        food_scoring: false,
        think_time: Duration::from_millis(think_time_ms),
        metrics: Mutex::new(Metrics::default()),
    };
    let guided = Variant {
        food_guidance: true,
        food_scoring: true,
        think_time: Duration::from_millis(think_time_ms),
        metrics: Mutex::new(Metrics::default()),
    };
    let config = GameConfig {
        max_turns,
        ..GameConfig::duel()
    };

    for offset in 0..seeds {
        let seed = first_seed + offset;
        for guided_seat in [0, 1] {
            baseline.reset_metrics();
            guided.reset_metrics();
            let agents: [&dyn Agent; 2] = if guided_seat == 0 {
                [&guided, &baseline]
            } else {
                [&baseline, &guided]
            };
            let result = gym::runner::run_game_seeded(&agents, &config, seed);
            let outcome = result
                .winner
                .map(|winner| if winner == guided_seat { "win" } else { "loss" })
                .unwrap_or("draw");
            let guided_metrics = guided.metrics.lock().unwrap();
            let baseline_metrics = baseline.metrics.lock().unwrap();
            println!(
                "{}",
                serde_json::json!({
                    "pair_id": pair_id,
                    "seed": seed,
                    "guided_seat": guided_seat,
                    "budget_ms": think_time_ms,
                    "max_turns": max_turns,
                    "turns": result.turns,
                    "winner": result.winner,
                    "guided_outcome": outcome,
                    "guided_moves": guided_metrics.moves,
                    "guided_food_eaten": guided_metrics.food_eaten,
                    "guided_moves_onto_food": guided_metrics.moves_onto_food,
                    "guided_moves_closer_to_food": guided_metrics.moves_closer_to_food,
                    "baseline_moves": baseline_metrics.moves,
                    "baseline_food_eaten": baseline_metrics.food_eaten,
                    "baseline_moves_onto_food": baseline_metrics.moves_onto_food,
                    "baseline_moves_closer_to_food": baseline_metrics.moves_closer_to_food,
                })
            );
        }
        eprintln!("{pair_id}: {}/{} seeds", offset + 1, seeds);
    }
}
