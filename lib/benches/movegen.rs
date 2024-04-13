//! This is the benchmarking setup for bene-snake. It is used to measure the performance of the
//! move generation and evaluation functions.
//! It is important to note that you have to use the --feature bench.
//! The following metrics are important for benchmarking:
//! - Latency: The time it takes to generate a move
//! - Allocations: The allocations done by a move generation
use battlesnake_game_types::compact_representation::StandardCellBoard4Snakes11x11;
use battlesnake_game_types::types::{
    build_snake_id_map, SimulableGame, SnakeIDGettableGame, SnakeIDMap, YouDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use divan::AllocProfiler;
use lib::{decode_state, evaluate_board};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn gen_test_board(string: &mut str) -> (StandardCellBoard4Snakes11x11, SnakeIDMap) {
    let mut string_clone = string.to_string();
    let board: Game = unsafe { simd_json::from_str(string_clone.as_mut_str()).unwrap() };
    let mut hm = HashMap::new();
    let snake_id_map = build_snake_id_map(&board);
    hm.insert(board.game.id.to_string(), snake_id_map.clone());
    let g = decode_state(string.to_string(), Arc::new(Mutex::new(hm))).unwrap();
    (g, snake_id_map)
}

fn test_boards() -> Vec<(StandardCellBoard4Snakes11x11, SnakeIDMap)> {
    let mut boards = vec![];
    let input_strings = [
        r##"{"game":{"id":"3c542096-07eb-439b-aab7-0c2b9aaac58c","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_3fHj37VrM4xJBj8YTMc9B6gQ","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_3CVfr7jCv7T6SXwrp8hJrY39","name":"Snaker","latency":"","health":100,"body":[{"x":1,"y":1},{"x":1,"y":1},{"x":1,"y":1}],"head":{"x":1,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}},{"id":"gs_m8hBvHF4pB8Ybdb6FYVpPdSG","name":"DefenderSnake","latency":"","health":100,"body":[{"x":9,"y":9},{"x":9,"y":9},{"x":9,"y":9}],"head":{"x":9,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff0000","head":"tongue","tail":"weight"}},{"id":"gs_9qTMgCbQRxX8VDwbHD79xb3X","name":"new myfirstSnake();","latency":"","health":100,"body":[{"x":9,"y":1},{"x":9,"y":1},{"x":9,"y":1}],"head":{"x":9,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}],"food":[{"x":2,"y":10},{"x":2,"y":0},{"x":8,"y":10},{"x":10,"y":2},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_3fHj37VrM4xJBj8YTMc9B6gQ","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"7417b69a-bbe9-47f3-b88b-db0e7e33cd48","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":51,"board":{"height":11,"width":11,"snakes":[{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RpJkFVGrG6W68bhQMxp6G738","name":"Hungry Bot","latency":"1","health":97,"body":[{"x":7,"y":0},{"x":6,"y":0},{"x":5,"y":0},{"x":4,"y":0},{"x":4,"y":1},{"x":4,"y":2},{"x":3,"y":2},{"x":2,"y":2},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":3}],"head":{"x":7,"y":0},"length":11,"shout":"","squad":"","customizations":{"color":"#00cc00","head":"alligator","tail":"alligator"}}],"food":[{"x":7,"y":9}],"hazards":[]},"you":{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"367e5029-1e66-42dc-9dc7-651da8314602","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":96,"board":{"height":11,"width":11,"snakes":[{"id":"gs_7GddFryxCkcH4mTbG9PwFYPf","name":"bene-snake-dev","latency":"32","health":88,"body":[{"x":4,"y":2},{"x":4,"y":3},{"x":3,"y":3},{"x":2,"y":3},{"x":1,"y":3},{"x":1,"y":2},{"x":1,"y":1},{"x":2,"y":1},{"x":3,"y":1},{"x":4,"y":1}],"head":{"x":4,"y":2},"length":10,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_CHSHSf6fWSPPwfVWgJ6Q4VYb","name":"Snaker","latency":"79","health":68,"body":[{"x":0,"y":8},{"x":0,"y":7},{"x":0,"y":6},{"x":1,"y":6},{"x":1,"y":7}],"head":{"x":0,"y":8},"length":5,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}}],"food":[{"x":4,"y":0},{"x":8,"y":7},{"x":10,"y":8},{"x":4,"y":5},{"x":5,"y":10},{"x":3,"y":8},{"x":7,"y":7},{"x":10,"y":10},{"x":2,"y":2},{"x":2,"y":8},{"x":3,"y":2}],"hazards":[]},"you":{"id":"gs_7GddFryxCkcH4mTbG9PwFYPf","name":"bene-snake-dev","latency":"32","health":88,"body":[{"x":4,"y":2},{"x":4,"y":3},{"x":3,"y":3},{"x":2,"y":3},{"x":1,"y":3},{"x":1,"y":2},{"x":1,"y":1},{"x":2,"y":1},{"x":3,"y":1},{"x":4,"y":1}],"head":{"x":4,"y":2},"length":10,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"4602cd9e-16e7-4dab-a0d1-ee328a2cacf0","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_qQmh66fjt8bVDF9PkrCFB3Pb","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_8DhmJ3Hk7SM434G4fpPQcKgV","name":"Snaker","latency":"","health":100,"body":[{"x":9,"y":1},{"x":9,"y":1},{"x":9,"y":1}],"head":{"x":9,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}},{"id":"gs_DgFCtmtHRqQvhWGhJBvdVgtJ","name":"DefenderSnake","latency":"","health":100,"body":[{"x":9,"y":9},{"x":9,"y":9},{"x":9,"y":9}],"head":{"x":9,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff0000","head":"tongue","tail":"weight"}},{"id":"gs_bJJ8MMfjxxPYmm7vcfb8SCpc","name":"new myfirstSnake();","latency":"","health":100,"body":[{"x":1,"y":1},{"x":1,"y":1},{"x":1,"y":1}],"head":{"x":1,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}],"food":[{"x":2,"y":10},{"x":8,"y":0},{"x":8,"y":10},{"x":0,"y":2},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_qQmh66fjt8bVDF9PkrCFB3Pb","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"6b899de0-d092-4ec5-a60b-d18135cfc885","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":28,"board":{"height":11,"width":11,"snakes":[{"id":"gs_bVYpC8MCxmgtJtyFr9THJRYK","name":"bene-snake-dev","latency":"50","health":98,"body":[{"x":1,"y":7},{"x":0,"y":7},{"x":0,"y":6},{"x":0,"y":5},{"x":1,"y":5},{"x":1,"y":6}],"head":{"x":1,"y":7},"length":6,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_pwcJv9xbSrFq49RrhQTYcWjT","name":"Snaker","latency":"80","health":72,"body":[{"x":1,"y":3},{"x":0,"y":3},{"x":0,"y":4}],"head":{"x":1,"y":3},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}}],"food":[{"x":4,"y":0},{"x":10,"y":4},{"x":6,"y":3},{"x":0,"y":1},{"x":8,"y":8},{"x":2,"y":6}],"hazards":[]},"you":{"id":"gs_bVYpC8MCxmgtJtyFr9THJRYK","name":"bene-snake-dev","latency":"50","health":98,"body":[{"x":1,"y":7},{"x":0,"y":7},{"x":0,"y":6},{"x":0,"y":5},{"x":1,"y":5},{"x":1,"y":6}],"head":{"x":1,"y":7},"length":6,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"30098c6d-af86-40c2-8bbe-03233a53561e","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"ladder"},"turn":71,"board":{"height":11,"width":11,"snakes":[{"id":"gs_kwpxv947rRp3hr6kYdVvm8MR","name":"bene-snake","latency":"444","health":45,"body":[{"x":9,"y":2},{"x":10,"y":2},{"x":10,"y":1},{"x":9,"y":1},{"x":8,"y":1}],"head":{"x":9,"y":2},"length":5,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}},{"id":"gs_pMkv4SW8QrrxyppxTxwmFpqK","name":"transferred-vercel-attempt","latency":"28","health":92,"body":[{"x":4,"y":7},{"x":5,"y":7},{"x":5,"y":6},{"x":5,"y":5},{"x":6,"y":5},{"x":7,"y":5}],"head":{"x":4,"y":7},"length":6,"shout":"","squad":"","customizations":{"color":"#2e77ff","head":"nr-rocket","tail":"bolt"}}],"food":[{"x":10,"y":6},{"x":1,"y":7},{"x":0,"y":10},{"x":2,"y":6},{"x":8,"y":9}],"hazards":[]},"you":{"id":"gs_kwpxv947rRp3hr6kYdVvm8MR","name":"bene-snake","latency":"444","health":45,"body":[{"x":9,"y":2},{"x":10,"y":2},{"x":10,"y":1},{"x":9,"y":1},{"x":8,"y":1}],"head":{"x":9,"y":2},"length":5,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}}"##,
        r##"{"game":{"id":"5b7f08e2-3a38-4285-b461-ac01481686de","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"ladder"},"turn":200,"board":{"height":11,"width":11,"snakes":[{"id":"gs_F7Rt3mVCQyF9btj7CgD6Wm6Q","name":"bene-snake","latency":"443","health":79,"body":[{"x":4,"y":10},{"x":3,"y":10},{"x":2,"y":10},{"x":1,"y":10},{"x":0,"y":10},{"x":0,"y":9},{"x":1,"y":9},{"x":2,"y":9},{"x":3,"y":9},{"x":4,"y":9},{"x":5,"y":9},{"x":6,"y":9},{"x":7,"y":9},{"x":8,"y":9}],"head":{"x":4,"y":10},"length":14,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}},{"id":"gs_W4HthT9VDFPmTrG774cPTmCV","name":"coffee lover","latency":"39","health":86,"body":[{"x":2,"y":6},{"x":2,"y":7},{"x":2,"y":8},{"x":1,"y":8},{"x":1,"y":7},{"x":0,"y":7},{"x":0,"y":6},{"x":1,"y":6},{"x":1,"y":5},{"x":1,"y":4},{"x":1,"y":3},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":1},{"x":0,"y":0},{"x":1,"y":0},{"x":1,"y":1},{"x":2,"y":1},{"x":2,"y":2},{"x":3,"y":2},{"x":4,"y":2},{"x":5,"y":2},{"x":6,"y":2},{"x":7,"y":2},{"x":8,"y":2},{"x":8,"y":3}],"head":{"x":2,"y":6},"length":26,"shout":"No coffee? Then I want food!","squad":"","customizations":{"color":"#175b8c","head":"caffeine","tail":"coffee"}}],"food":[{"x":4,"y":0},{"x":3,"y":0}],"hazards":[]},"you":{"id":"gs_F7Rt3mVCQyF9btj7CgD6Wm6Q","name":"bene-snake","latency":"443","health":79,"body":[{"x":4,"y":10},{"x":3,"y":10},{"x":2,"y":10},{"x":1,"y":10},{"x":0,"y":10},{"x":0,"y":9},{"x":1,"y":9},{"x":2,"y":9},{"x":3,"y":9},{"x":4,"y":9},{"x":5,"y":9},{"x":6,"y":9},{"x":7,"y":9},{"x":8,"y":9}],"head":{"x":4,"y":10},"length":14,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}}"##,
        r##"{"game":{"id":"ac45448a-c4d1-447f-9664-e0be5d7dc944","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"ladder"},"turn":118,"board":{"height":11,"width":11,"snakes":[{"id":"gs_p8R9XwDY9bx4JW8KMkBG9PMb","name":"Warmsnake","latency":"9","health":95,"body":[{"x":6,"y":4},{"x":5,"y":4},{"x":5,"y":5},{"x":4,"y":5},{"x":4,"y":4},{"x":3,"y":4},{"x":3,"y":5},{"x":3,"y":6},{"x":3,"y":7},{"x":3,"y":8},{"x":2,"y":8}],"head":{"x":6,"y":4},"length":11,"shout":"","squad":"","customizations":{"color":"#abcdef","head":"default","tail":"default"}},{"id":"gs_dGJTwMFDhHCk3GkQkMCTrMY6","name":"bene-snake","latency":"443","health":53,"body":[{"x":9,"y":1},{"x":8,"y":1},{"x":7,"y":1},{"x":6,"y":1},{"x":5,"y":1},{"x":4,"y":1},{"x":3,"y":1},{"x":2,"y":1},{"x":1,"y":1},{"x":0,"y":1}],"head":{"x":9,"y":1},"length":10,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}],"food":[{"x":6,"y":10},{"x":8,"y":10},{"x":9,"y":8},{"x":9,"y":4},{"x":6,"y":5},{"x":1,"y":0}],"hazards":[]},"you":{"id":"gs_dGJTwMFDhHCk3GkQkMCTrMY6","name":"bene-snake","latency":"443","health":53,"body":[{"x":9,"y":1},{"x":8,"y":1},{"x":7,"y":1},{"x":6,"y":1},{"x":5,"y":1},{"x":4,"y":1},{"x":3,"y":1},{"x":2,"y":1},{"x":1,"y":1},{"x":0,"y":1}],"head":{"x":9,"y":1},"length":10,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}}"##,
    ];
    for input_string in input_strings.iter() {
        boards.push(gen_test_board(&mut input_string.to_string()));
    }
    boards
}

fn main() {
    divan::main();
}

#[divan::bench]
fn test_calc_move_depth_3() {
    let boards = test_boards();
    for board in boards.iter() {
        divan::black_box(lib::calc_move(board.0, 3, Instant::now()));
    }
}

#[divan::bench]
fn test_calc_move_depth_4() {
    let boards = test_boards();
    for board in boards.iter() {
        divan::black_box(lib::calc_move(board.0, 4, Instant::now()));
    }
}

// Simulate a real game, where caching could be effective
#[divan::bench]
fn test_calc_moves_sequential_boards() {
    let boards = test_boards();
    for mut board in boards.iter().map(|x| x.0) {
        for _ in 0..3 {
            divan::black_box(lib::calc_move(board, 3, Instant::now()));

            board = divan::black_box(
                board
                    .clone()
                    .simulate(&lib::Simulator {}, board.get_snake_ids().to_vec())
                    .next()
                    .unwrap()
                    .1,
            );
        }
    }
}

#[divan::bench]
fn bench_eval() {
    let boards = test_boards();
    for board in boards.iter() {
        divan::black_box(evaluate_board(
            board.0,
            board.0.you_id(),
            Cow::Owned(board.1.values().copied().collect()),
        ));
    }
}

#[tokio::main]
#[divan::bench]
async fn bench_with_tokio() {
    bench_eval();
    test_calc_moves_sequential_boards();
    test_calc_move_depth_3();
    test_calc_move_depth_4();
}
