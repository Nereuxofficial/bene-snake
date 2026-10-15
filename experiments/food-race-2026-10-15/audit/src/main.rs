use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
use lib::{
    food_race::{MAX_EXPANSIONS, TargetHistory, assess},
    mcts::{Node, SearchDepthStats, search_once_with_rng},
};
use rand::SeedableRng;
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
fn point(v: &Value) -> Value {
    json!({"x":v["X"],"y":v["Y"]})
}
fn main() {
    let rootdir = std::env::args().nth(1).unwrap();
    let search = std::env::args().any(|a| a == "--search");
    let rows: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{rootdir}/rows.json")).unwrap())
            .unwrap();
    for row in rows.as_array().unwrap() {
        let gid = row["game_id"].as_str().unwrap();
        if search && !gid.starts_with("e6bcd8f8") {
            continue;
        }
        let p: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{rootdir}/{gid}.json")).unwrap(),
        )
        .unwrap();
        let fs = p["frames"].as_array().unwrap();
        let mut history = TargetHistory::default();
        for (i, f) in fs.iter().enumerate().take(fs.len() - 1) {
            if search && !(275..=281).contains(&i) {
                continue;
            }
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
            let game:Game=serde_json::from_value(json!({"game":{"id":gid,"timeout":500,"ruleset":{"name":"standard","version":"1"}},"turn":f["Turn"],"board":{"width":11,"height":11,"food":f["Food"].as_array().unwrap().iter().map(point).collect::<Vec<_>>(),"hazards":[],"snakes":snakes},"you":snakes[0]})).unwrap();
            let ids = build_snake_id_map(&game);
            let board = game.as_cell_board(&ids).unwrap();
            let you = ids[&game.you.id];
            let eval = |enabled| {
                let start = Instant::now();
                let root = Arc::new(Node::new_root_with_food_races(board, enabled));
                root.prefer_food_target(you, history.hint(game.turn));
                let candidates = root.root_candidates(you).unwrap();
                let ms = start.elapsed().as_secs_f64() * 1000.;
                (root, candidates, ms)
            };
            let (old, new) = if i % 2 == 0 {
                let a = eval(false);
                let b = eval(true);
                (a, b)
            } else {
                let b = eval(true);
                let a = eval(false);
                (a, b)
            };
            assert_eq!(old.1.0, new.1.0, "race prior must never prune");
            for idx in 0..4 {
                if new.1.0 & (1 << idx) != 0 {
                    assert!(new.1.1[idx] > 0);
                }
            }
            history.observe(game.turn, new.0.food_target());
            let a = new.0.root_food_race(you).unwrap();
            assert!(a.expansions <= MAX_EXPANSIONS);
            let food:Vec<_>=a.foods.iter().map(|f|json!({"cell":f.cell,"class":format!("{:?}",f.class),"arrival":f.arrival,"rival_arrival":f.rival_arrival,"rival_length":f.rival_length,"uncertain":f.uncertain_route,"first":f.first_moves})).collect();
            let deterministic = assess(&board, you, new.1.0, None, MAX_EXPANSIONS);
            if !a.budget_exhausted {
                assert_eq!(
                    format!("{:?}", a.foods),
                    format!("{:?}", deterministic.foods)
                );
            }
            let mut decisions = Vec::new();
            if search {
                for seed in 0..4 {
                    let mut pair = Vec::new();
                    for enabled in [false, true] {
                        let r = Arc::new(Node::new_root_with_food_races(board, enabled));
                        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
                        let mut stats = SearchDepthStats::default();
                        for _ in 0..20_000 {
                            search_once_with_rng(&r, &you, &mut stats, &mut rng);
                        }
                        pair.push(r.best_move(you).map(|mv| mv.to_string()));
                    }
                    decisions.push(pair);
                }
            }
            println!(
                "{}",
                json!({"game_id":gid,"turn":game.turn,"old_mask":old.1.0,"new_mask":new.1.0,"old_weights":old.1.1,"new_weights":new.1.1,"old_ms":old.2,"new_ms":new.2,"target":new.0.food_target(),"exhausted":a.budget_exhausted,"expansions":a.expansions,"best_chance":a.best_chance,"foods":food,"decisions":decisions})
            );
        }
    }
}
