use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
use lib::{
    enclosure::assess,
    eval::evaluate_board_with_options,
    mcts::{Node, SearchDepthStats, search_for, search_once_with_rng},
};
use rand::SeedableRng;
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn main() {
    let games: Vec<Game> = serde_json::from_str(include_str!(
        "../../../../lib/fixtures/enclosure-replays.json"
    ))
    .unwrap();
    let mode = std::env::args().nth(1).unwrap_or("audit".into());
    for game in games {
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).unwrap();
        let you = ids[&game.you.id];
        let a = assess(&board, you);
        if mode == "audit" {
            let start = Instant::now();
            for _ in 0..1000 {
                std::hint::black_box(assess(std::hint::black_box(&board), you));
            }
            println!(
                "{}",
                json!({"game":game.game.id,"turn":game.turn,"usable":a.usable,"open":a.open,"gates":a.contested_exits,"uncertain":a.uncertain,"penalty":a.penalty,"old":evaluate_board_with_options(&board,&you,true,false),"new":evaluate_board_with_options(&board,&you,true,true),"mean_us":start.elapsed().as_secs_f64()*1000.})
            );
            continue;
        }
        if !(game.turn % 4 == 0 || (mode == "calibrate" && game.turn == 228)) {
            continue;
        }
        for seed in 0..if mode == "calibrate" { 1 } else { 3 } {
            for enabled in if seed % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = Instant::now();
                let root = Arc::new(Node::new_root_with_enclosure_evaluation(board, enabled));
                let candidates = root.root_candidates(you);
                let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
                let mut stats = SearchDepthStats::default();
                let mut n = 0;
                if mode == "parallel" {
                    let remaining = Duration::from_millis(350).saturating_sub(start.elapsed());
                    stats = search_for(&root, &you, remaining, 12);
                    n = stats.iterations;
                } else {
                    loop {
                        if mode == "fixed" {
                            if n >= 10000 {
                                break;
                            }
                        } else if start.elapsed() >= Duration::from_millis(60) {
                            break;
                        }
                        search_once_with_rng(&root, &you, &mut stats, &mut rng);
                        n += 1;
                    }
                }
                println!(
                    "{}",
                    json!({"game":game.game.id,"turn":game.turn,"seed":if mode=="parallel"{None}else{Some(seed)},"repeat":seed,"enabled":enabled,"mode":mode,"iterations":n,"ms":start.elapsed().as_secs_f64()*1000.,"move":root.best_move(you).map(|m|m.to_string()),"candidates":candidates})
                );
            }
        }
        if mode == "calibrate" {
            break;
        }
    }
}
