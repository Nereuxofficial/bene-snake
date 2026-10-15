use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
use lib::{
    eval::evaluate_board_with_food_races,
    food_race::{MAX_EXPANSIONS, TargetHistory, assess},
    mcts::{Node, SearchDepthStats, search_for, search_once_with_rng},
};
use rand::SeedableRng;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn point(v: &Value) -> Value {
    json!({"x":v["X"],"y":v["Y"]})
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let dir = &args[1];
    let iterations: usize = args[2].parse().unwrap();
    let ms: u64 = args[3].parse().unwrap();
    let seeds: u64 = args[4].parse().unwrap();
    let stride: usize = args[5].parse().unwrap();
    for (gid, window) in [
        ("619cc13e-216b-4aab-814f-417b87acc3b2", 66..=74),
        ("dda46e06-1f9f-461f-a3af-1f0e6476af0f", 185..=193),
        ("e6bcd8f8-8cf2-4474-93c7-2e773945b53f", 274..=282),
    ] {
        let replay: Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{dir}/{gid}.json")).unwrap())
                .unwrap();
        let mut history = TargetHistory::default();
        let mut trigger_history = lib::tactical::TriggerHistory::default();
        for f in replay["frames"].as_array().unwrap() {
            let turn = f["Turn"].as_i64().unwrap() as i32;
            let us = f["Snakes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["Name"] == "bene-snake")
                .unwrap();
            if !us["Death"].is_null() {
                break;
            }
            let mut snakes:Vec<Value>=f["Snakes"].as_array().unwrap().iter().filter(|s|s["Death"].is_null()).map(|s|json!({"id":s["ID"],"name":s["Name"],"health":s["Health"],"body":s["Body"].as_array().unwrap().iter().map(point).collect::<Vec<_>>(),"head":point(&s["Body"][0])})).collect();
            snakes.sort_by_key(|s| s["name"] != "bene-snake");
            let game:Game=serde_json::from_value(json!({"game":{"id":gid,"timeout":500,"ruleset":{"name":"standard","version":"1"}},"turn":turn,"board":{"width":11,"height":11,"food":f["Food"].as_array().unwrap().iter().map(point).collect::<Vec<_>>(),"hazards":f["Hazards"].as_array().unwrap().iter().map(point).collect::<Vec<_>>(),"snakes":snakes},"you":snakes[0]})).unwrap();
            let ids = build_snake_id_map(&game);
            let board = game.as_cell_board(&ids).unwrap();
            let you = ids[&game.you.id];
            let hint = history.hint(turn);
            let root = Node::new_root(board);
            root.prefer_food_target(you, hint);
            history.observe(turn, root.food_target());
            let filter = if iterations == 0 {
                let mut limits = lib::tactical::Limits::default();
                limits.deadline = None;
                let filter = root.tactical_root_filter_with_continuation(
                    you,
                    &limits,
                    trigger_history.should_continue(turn),
                );
                trigger_history.observe(turn, filter.applied);
                Some(filter)
            } else {
                None
            };
            let selected =
                window.contains(&turn) && (turn - *window.start()) as usize % stride == 0;
            let start = Instant::now();
            let mut score = 0;
            for _ in 0..100 {
                score = std::hint::black_box(evaluate_board_with_food_races(
                    std::hint::black_box(&board),
                    &you,
                    true,
                ));
            }
            let eval_ns = start.elapsed().as_nanos() / 100;
            let old_score = evaluate_board_with_food_races(&board, &you, false);
            let a = assess(&board, you, 15, None, MAX_EXPANSIONS);
            let mut runs = vec![];
            if selected {
                for seed in 0..seeds {
                    for mode in ["iterations", "time"] {
                        for enabled in if (seed + turn as u64) % 2 == 0 {
                            [false, true]
                        } else {
                            [true, false]
                        } {
                            let start = Instant::now();
                            let r =
                                Arc::new(Node::new_root_with_food_race_evaluation(board, enabled));
                            r.prefer_food_target(you, hint);
                            assert_eq!(r.root_candidates(you), root.root_candidates(you));
                            let mut stats = SearchDepthStats::default();
                            if mode == "iterations" {
                                let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
                                for _ in 0..iterations {
                                    search_once_with_rng(&r, &you, &mut stats, &mut rng);
                                }
                            } else {
                                stats = search_for(
                                    &r,
                                    &you,
                                    Duration::from_millis(ms).saturating_sub(start.elapsed()),
                                    12,
                                );
                            }
                            let elapsed = start.elapsed().as_secs_f64() * 1000.;
                            runs.push(json!({"seed":seed,"mode":mode,"enabled":enabled,"move":r.best_move(you).map(|m|m.to_string()),"visits":r.visits(),"iterations":stats.iterations,"ms":elapsed}));
                        }
                    }
                }
            }
            println!(
                "{}",
                json!({"game":gid,"turn":turn,"health":us["Health"],"head":us["Body"][0],"old_score":old_score,"score":score,"eval_ns":eval_ns,"exhausted":a.budget_exhausted,"expansions":a.expansions,"root_mask":root.root_candidates(you).unwrap().0,"filter_mask":filter.as_ref().map(|f|f.mask),"foods":format!("{:?}",a.foods),"runs":runs})
            );
        }
    }
}
