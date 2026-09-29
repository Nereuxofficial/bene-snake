#![feature(iter_collect_into)]
#![allow(dead_code)]

use std::{
    sync::{
        Arc,
        Mutex,
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
    food_scoring: bool,
    think_time: Duration,
    diagnostics: Mutex<Diagnostics>,
}

#[derive(Default)]
struct Diagnostics {
    last_length: Option<u16>,
    food_eaten: u32,
    moves_onto_food: u32,
    moves_closer_to_food: u32,
}

impl Variant {
    fn reset_diagnostics(&self) {
        *self.diagnostics.lock().unwrap() = Diagnostics::default();
    }
}

impl Agent for Variant {
    fn name(&self) -> &str {
        if self.food_scoring {
            "food-weight-12"
        } else {
            "baseline"
        }
    }

    fn choose_move(&self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Move {
        let head = board.get_head_as_position(&you);
        let current_food_distance = board
            .get_all_food_as_positions()
            .iter()
            .map(|food| (head.x - food.x).abs() + (head.y - food.y).abs())
            .min();
        let current_length = board.get_length(&you);
        let root = Arc::new(mcts::Node::new_root(*board));
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
        let food = board.get_all_food_as_positions();
        let moves_onto_food = food.contains(&destination);
        let moves_closer_to_food = current_food_distance.is_some_and(|distance| {
            food.iter()
                .map(|food| {
                    (destination.x - food.x).abs() + (destination.y - food.y).abs()
                })
                .min()
                .is_some_and(|next| next < distance)
        });
        let mut diagnostics = self.diagnostics.lock().unwrap();
        if diagnostics
            .last_length
            .is_some_and(|last_length| current_length > last_length)
        {
            diagnostics.food_eaten += u32::from(current_length - diagnostics.last_length.unwrap());
        }
        diagnostics.last_length = Some(current_length);
        diagnostics.moves_onto_food += u32::from(moves_onto_food);
        diagnostics.moves_closer_to_food += u32::from(moves_closer_to_food);
        chosen
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 6,
        "usage: food-study SEED_COUNT MS FIRST_SEED PAIR_ID MAX_TURNS"
    );
    let seeds: u64 = args[1].parse().unwrap();
    let think_time_ms: u64 = args[2].parse().unwrap();
    let first_seed: u64 = args[3].parse().unwrap();
    let pair_id = &args[4];
    let max_turns: u32 = args[5].parse().unwrap();
    assert!(seeds > 0 && think_time_ms > 0);

    let baseline = Variant {
        food_scoring: false,
        think_time: Duration::from_millis(think_time_ms),
        diagnostics: Mutex::new(Diagnostics::default()),
    };
    let food = Variant {
        food_scoring: true,
        think_time: Duration::from_millis(think_time_ms),
        diagnostics: Mutex::new(Diagnostics::default()),
    };
    let config = GameConfig {
        max_turns,
        ..GameConfig::duel()
    };

    for offset in 0..seeds {
        let seed = first_seed + offset;
        for food_seat in [0, 1] {
            food.reset_diagnostics();
            baseline.reset_diagnostics();
            let agents: [&dyn Agent; 2] = if food_seat == 0 {
                [&food, &baseline]
            } else {
                [&baseline, &food]
            };
            let result = gym::runner::run_game_seeded(&agents, &config, seed);
            let outcome = result
                .winner
                .map(|winner| if winner == food_seat { "win" } else { "loss" })
                .unwrap_or("draw");
            let food_diagnostics = food.diagnostics.lock().unwrap();
            let baseline_diagnostics = baseline.diagnostics.lock().unwrap();
            println!(
                "{}",
                serde_json::json!({
                    "pair_id": pair_id,
                    "seed": seed,
                    "food_seat": food_seat,
                    "budget_ms": think_time_ms,
                    "max_turns": max_turns,
                    "turns": result.turns,
                    "winner": result.winner,
                    "food_outcome": outcome,
                    "food_eaten": food_diagnostics.food_eaten,
                    "food_moves_onto_food": food_diagnostics.moves_onto_food,
                    "food_moves_closer": food_diagnostics.moves_closer_to_food,
                    "baseline_food_eaten": baseline_diagnostics.food_eaten,
                    "baseline_moves_onto_food": baseline_diagnostics.moves_onto_food,
                    "baseline_moves_closer": baseline_diagnostics.moves_closer_to_food,
                })
            );
        }
        eprintln!("{pair_id}: {}/{} seeds", offset + 1, seeds);
    }
}
