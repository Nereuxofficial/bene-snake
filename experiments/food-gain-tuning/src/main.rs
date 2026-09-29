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
mod mcts;

struct Variant {
    food_gain_reward: u32,
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
    early_food_eaten: u32,
    early_moves_onto_food: u32,
}

impl Variant {
    fn reset_metrics(&self) {
        *self.metrics.lock().unwrap() = Metrics::default();
    }
}

impl Agent for Variant {
    fn name(&self) -> &str {
        "food-weight-tuning"
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
                let eaten = u32::from(current_length.saturating_sub(last_length));
                metrics.food_eaten += eaten;
                if metrics.moves <= 25 {
                    metrics.early_food_eaten += eaten;
                }
            }
            metrics.last_length = Some(current_length);
        }
        let root = Arc::new(mcts::Node::new_root_with_food_gain_reward(
            *board,
            self.food_gain_reward,
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let search_root = Arc::clone(&root);
        let search_stop = Arc::clone(&stop);
        let search = thread::spawn(move || {
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
        if metrics.moves <= 25 {
            metrics.early_moves_onto_food += u32::from(moves_onto_food);
        }
        chosen
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() == 6,
        "usage: food-gain-tuning SEED MS FOOD_GAIN_REWARD MAX_TURNS RUN_ID"
    );
    let seed: u64 = args[1].parse().unwrap();
    let budget_ms: u64 = args[2].parse().unwrap();
    let reward: u32 = args[3].parse().unwrap();
    let max_turns: u32 = args[4].parse().unwrap();
    let run_id = &args[5];
    assert!(budget_ms > 0 && reward <= 200);

    let baseline = Variant {
        food_gain_reward: 0,
        think_time: Duration::from_millis(budget_ms),
        metrics: Mutex::new(Metrics::default()),
    };
    let candidate = Variant {
        food_gain_reward: reward,
        think_time: Duration::from_millis(budget_ms),
        metrics: Mutex::new(Metrics::default()),
    };
    let config = GameConfig {
        max_turns,
        ..GameConfig::duel()
    };

    for candidate_seat in [0, 1] {
        baseline.reset_metrics();
        candidate.reset_metrics();
        let agents: [&dyn Agent; 2] = if candidate_seat == 0 {
            [&candidate, &baseline]
        } else {
            [&baseline, &candidate]
        };
        let result = gym::runner::run_game_seeded(&agents, &config, seed);
        let outcome = result
            .winner
            .map(|winner| {
                if winner == candidate_seat {
                    "win"
                } else {
                    "loss"
                }
            })
            .unwrap_or("draw");
        let c = candidate.metrics.lock().unwrap();
        let b = baseline.metrics.lock().unwrap();
        println!(
            "{}",
            serde_json::json!({
                "run_id": run_id,
                "seed": seed,
                "candidate_seat": candidate_seat,
                "candidate_reward": reward,
                "baseline_reward": 0,
                "budget_ms": budget_ms,
                "max_turns": max_turns,
                "turns": result.turns,
                "candidate_outcome": outcome,
                "candidate_moves": c.moves,
                "candidate_food_eaten": c.food_eaten,
                "candidate_moves_onto_food": c.moves_onto_food,
                "candidate_moves_closer_to_food": c.moves_closer_to_food,
                "candidate_early_food_eaten": c.early_food_eaten,
                "candidate_early_moves_onto_food": c.early_moves_onto_food,
                "baseline_moves": b.moves,
                "baseline_food_eaten": b.food_eaten,
                "baseline_moves_onto_food": b.moves_onto_food,
                "baseline_moves_closer_to_food": b.moves_closer_to_food,
                "baseline_early_food_eaten": b.early_food_eaten,
                "baseline_early_moves_onto_food": b.early_moves_onto_food,
            })
        );
    }
}
