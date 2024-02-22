use battlesnake_game_types::compact_representation::StandardCellBoard4Snakes11x11;
use battlesnake_game_types::types::build_snake_id_map;
use battlesnake_game_types::wire_representation::Game;
use lib::decode_state;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn gen_test_board(string: &mut str) -> StandardCellBoard4Snakes11x11 {
    let mut string_clone = string.to_string();
    let board: Game = unsafe { simd_json::from_str(string_clone.as_mut_str()).unwrap() };
    let mut hm = HashMap::new();
    hm.insert(board.game.id.to_string(), build_snake_id_map(&board));
    let g = decode_state(string.to_string(), Arc::new(Mutex::new(hm)));
    g.unwrap()
}

fn test_boards() -> Vec<StandardCellBoard4Snakes11x11> {
    let mut boards = vec![];
    let input_strings = [
        r##"{"game":{"id":"3c542096-07eb-439b-aab7-0c2b9aaac58c","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_3fHj37VrM4xJBj8YTMc9B6gQ","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_3CVfr7jCv7T6SXwrp8hJrY39","name":"Snaker","latency":"","health":100,"body":[{"x":1,"y":1},{"x":1,"y":1},{"x":1,"y":1}],"head":{"x":1,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}},{"id":"gs_m8hBvHF4pB8Ybdb6FYVpPdSG","name":"DefenderSnake","latency":"","health":100,"body":[{"x":9,"y":9},{"x":9,"y":9},{"x":9,"y":9}],"head":{"x":9,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff0000","head":"tongue","tail":"weight"}},{"id":"gs_9qTMgCbQRxX8VDwbHD79xb3X","name":"new myfirstSnake();","latency":"","health":100,"body":[{"x":9,"y":1},{"x":9,"y":1},{"x":9,"y":1}],"head":{"x":9,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}],"food":[{"x":2,"y":10},{"x":2,"y":0},{"x":8,"y":10},{"x":10,"y":2},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_3fHj37VrM4xJBj8YTMc9B6gQ","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"7417b69a-bbe9-47f3-b88b-db0e7e33cd48","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":51,"board":{"height":11,"width":11,"snakes":[{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RpJkFVGrG6W68bhQMxp6G738","name":"Hungry Bot","latency":"1","health":97,"body":[{"x":7,"y":0},{"x":6,"y":0},{"x":5,"y":0},{"x":4,"y":0},{"x":4,"y":1},{"x":4,"y":2},{"x":3,"y":2},{"x":2,"y":2},{"x":1,"y":2},{"x":0,"y":2},{"x":0,"y":3}],"head":{"x":7,"y":0},"length":11,"shout":"","squad":"","customizations":{"color":"#00cc00","head":"alligator","tail":"alligator"}}],"food":[{"x":7,"y":9}],"hazards":[]},"you":{"id":"gs_RxF4j7TSMMPr3t4qSxSJyHjP","name":"bene-snake-dev","latency":"104","health":93,"body":[{"x":7,"y":4},{"x":6,"y":4},{"x":5,"y":4},{"x":4,"y":4},{"x":4,"y":5},{"x":5,"y":5},{"x":6,"y":5}],"head":{"x":7,"y":4},"length":7,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"367e5029-1e66-42dc-9dc7-651da8314602","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":96,"board":{"height":11,"width":11,"snakes":[{"id":"gs_7GddFryxCkcH4mTbG9PwFYPf","name":"bene-snake-dev","latency":"32","health":88,"body":[{"x":4,"y":2},{"x":4,"y":3},{"x":3,"y":3},{"x":2,"y":3},{"x":1,"y":3},{"x":1,"y":2},{"x":1,"y":1},{"x":2,"y":1},{"x":3,"y":1},{"x":4,"y":1}],"head":{"x":4,"y":2},"length":10,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_CHSHSf6fWSPPwfVWgJ6Q4VYb","name":"Snaker","latency":"79","health":68,"body":[{"x":0,"y":8},{"x":0,"y":7},{"x":0,"y":6},{"x":1,"y":6},{"x":1,"y":7}],"head":{"x":0,"y":8},"length":5,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}}],"food":[{"x":4,"y":0},{"x":8,"y":7},{"x":10,"y":8},{"x":4,"y":5},{"x":5,"y":10},{"x":3,"y":8},{"x":7,"y":7},{"x":10,"y":10},{"x":2,"y":2},{"x":2,"y":8},{"x":3,"y":2}],"hazards":[]},"you":{"id":"gs_7GddFryxCkcH4mTbG9PwFYPf","name":"bene-snake-dev","latency":"32","health":88,"body":[{"x":4,"y":2},{"x":4,"y":3},{"x":3,"y":3},{"x":2,"y":3},{"x":1,"y":3},{"x":1,"y":2},{"x":1,"y":1},{"x":2,"y":1},{"x":3,"y":1},{"x":4,"y":1}],"head":{"x":4,"y":2},"length":10,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"4602cd9e-16e7-4dab-a0d1-ee328a2cacf0","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_qQmh66fjt8bVDF9PkrCFB3Pb","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_8DhmJ3Hk7SM434G4fpPQcKgV","name":"Snaker","latency":"","health":100,"body":[{"x":9,"y":1},{"x":9,"y":1},{"x":9,"y":1}],"head":{"x":9,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}},{"id":"gs_DgFCtmtHRqQvhWGhJBvdVgtJ","name":"DefenderSnake","latency":"","health":100,"body":[{"x":9,"y":9},{"x":9,"y":9},{"x":9,"y":9}],"head":{"x":9,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff0000","head":"tongue","tail":"weight"}},{"id":"gs_bJJ8MMfjxxPYmm7vcfb8SCpc","name":"new myfirstSnake();","latency":"","health":100,"body":[{"x":1,"y":1},{"x":1,"y":1},{"x":1,"y":1}],"head":{"x":1,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}],"food":[{"x":2,"y":10},{"x":8,"y":0},{"x":8,"y":10},{"x":0,"y":2},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_qQmh66fjt8bVDF9PkrCFB3Pb","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
        r##"{"game":{"id":"6b899de0-d092-4ec5-a60b-d18135cfc885","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":28,"board":{"height":11,"width":11,"snakes":[{"id":"gs_bVYpC8MCxmgtJtyFr9THJRYK","name":"bene-snake-dev","latency":"50","health":98,"body":[{"x":1,"y":7},{"x":0,"y":7},{"x":0,"y":6},{"x":0,"y":5},{"x":1,"y":5},{"x":1,"y":6}],"head":{"x":1,"y":7},"length":6,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_pwcJv9xbSrFq49RrhQTYcWjT","name":"Snaker","latency":"80","health":72,"body":[{"x":1,"y":3},{"x":0,"y":3},{"x":0,"y":4}],"head":{"x":1,"y":3},"length":3,"shout":"","squad":"","customizations":{"color":"#0040ff","head":"earmuffs","tail":"bolt"}}],"food":[{"x":4,"y":0},{"x":10,"y":4},{"x":6,"y":3},{"x":0,"y":1},{"x":8,"y":8},{"x":2,"y":6}],"hazards":[]},"you":{"id":"gs_bVYpC8MCxmgtJtyFr9THJRYK","name":"bene-snake-dev","latency":"50","health":98,"body":[{"x":1,"y":7},{"x":0,"y":7},{"x":0,"y":6},{"x":0,"y":5},{"x":1,"y":5},{"x":1,"y":6}],"head":{"x":1,"y":7},"length":6,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##,
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
        lib::calc_move(*board, 3);
    }
}

#[divan::bench]
fn test_calc_move_depth_4() {
    let boards = test_boards();
    for board in boards.iter() {
        lib::calc_move(*board, 4);
    }
}
