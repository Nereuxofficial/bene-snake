use gym::{rules::*, schema::*};
use std::collections::BTreeMap;
fn p(x: i32, y: i32) -> Point {
    Point::new(x, y)
}
fn snake(id: &str, body: &[(i32, i32)], h: i32) -> Snake {
    Snake::new(id, body.iter().map(|&(x, y)| p(x, y)).collect(), h)
}
fn req() -> Request {
    Request::new(
        vec![
            snake("other", &[(8, 8), (8, 7), (8, 6)], 50),
            snake("focal", &[(1, 1), (1, 0), (0, 0)], 50),
        ],
        vec![],
        "focal",
    )
}
fn step(r: &Request, a: Move, b: Move) -> Transition {
    advance(
        &State::from_request(r),
        &BTreeMap::from([("focal".into(), a), ("other".into(), b)]),
        "focal",
        &Objective::Survive,
    )
}
#[test]
fn wall_reverse_and_body() {
    let r = req();
    assert!(
        step(&r, Move::Down, Move::Up)
            .eliminated
            .iter()
            .any(|e| e.snake.id == "focal")
    );
    let mut r = req();
    r.you = snake("focal", &[(0, 1), (1, 1), (1, 0)], 50);
    r.board.snakes[1] = r.you.clone();
    assert_eq!(step(&r, Move::Left, Move::Up).eliminated[0].cause, "wall");
}
#[test]
fn health_one_food_and_starvation() {
    let mut r = req();
    r.you.health = 1;
    r.board.snakes[1] = r.you.clone();
    assert!(
        !step(&r, Move::Up, Move::Up)
            .state
            .snakes
            .iter()
            .any(|s| s.id == "focal")
    );
    r.board.food.push(p(1, 2));
    let t = step(&r, Move::Up, Move::Up);
    let s = t.state.snakes.iter().find(|s| s.id == "focal").unwrap();
    assert_eq!((s.health, s.length), (100, 4));
    assert_eq!(s.body[2], s.body[3]);
}
#[test]
fn food_contest_equal_and_unequal() {
    let a = snake("focal", &[(3, 3), (2, 3), (1, 3)], 50);
    let b = snake("other", &[(5, 3), (6, 3), (7, 3)], 50);
    let mut r = Request::new(vec![a, b], vec![p(4, 3)], "focal");
    assert!(step(&r, Move::Right, Move::Left).state.snakes.is_empty());
    r.board.snakes[1].body.push(p(8, 3));
    r.board.snakes[1].length = 4;
    let t = step(&r, Move::Right, Move::Left);
    assert_eq!(t.state.snakes.len(), 1);
    assert_eq!(t.state.snakes[0].id, "other");
    assert_eq!(t.state.snakes[0].length, 5);
}
#[test]
fn vacating_and_stacked_tail() {
    let a = snake("focal", &[(2, 2), (2, 1), (1, 1), (1, 2)], 50);
    let b = snake("other", &[(8, 8), (8, 7), (8, 6)], 50);
    let mut r = Request::new(vec![a, b], vec![], "focal");
    assert!(
        step(&r, Move::Left, Move::Up)
            .state
            .snakes
            .iter()
            .any(|s| s.id == "focal")
    );
    r.you.body.push(p(1, 2));
    r.you.length += 1;
    r.board.snakes[0] = r.you.clone();
    assert!(
        step(&r, Move::Left, Move::Up)
            .eliminated
            .iter()
            .any(|s| s.snake.id == "focal")
    );
}
#[test]
fn validation_nonzero_wire_id_and_rejections() {
    let r = req();
    validate(&r).unwrap();
    assert_eq!(r.board.snakes[1].id, r.you.id);
    let mut bad = r.clone();
    bad.board.width = 12;
    assert!(validate(&bad).is_err());
    bad = r.clone();
    bad.game.ruleset.name = "wrapped".into();
    assert!(validate(&bad).is_err());
    bad = r.clone();
    bad.you.length = 4;
    assert!(validate(&bad).is_err());
    bad = r.clone();
    bad.board.food.push(bad.you.head);
    assert!(validate(&bad).is_err());
    bad = r.clone();
    bad.board.snakes[0].body[1] = p(0, 9);
    assert!(validate(&bad).is_err());
}
#[test]
fn objective_progress_and_extinction() {
    let r = req();
    let mut s = State::from_request(&r);
    assert_eq!(
        s.terminal("focal", &Objective::Survive, 0).unwrap().0,
        Label::Success
    );
    assert_eq!(
        s.terminal("focal", &Objective::SoleSurvivor, 0).unwrap().0,
        Label::Failure
    );
    s.snakes.clear();
    assert_eq!(
        s.terminal("focal", &Objective::Survive, 0).unwrap().0,
        Label::Failure
    );
}
#[test]
fn compact_differential_generated_transitions() {
    use battlesnake_game_types::{
        compact_representation::standard::CellBoard4Snakes11x11, types::*,
        wire_representation::Game,
    };
    for x in 2..8 {
        for y in 2..8 {
            let r = Request::new(
                vec![
                    snake("focal", &[(x, y), (x - 1, y), (x - 2, y)], 70),
                    snake("other", &[(9, 9), (9, 8), (9, 7)], 60),
                ],
                vec![p(x, y + 1)],
                "focal",
            );
            validate(&r).unwrap();
            let g: Game = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
            let ids = build_snake_id_map(&g);
            let c: CellBoard4Snakes11x11 = g.as_cell_board(&ids).unwrap();
            for mv in gym::schema::Move::ALL {
                let cm = match mv {
                    gym::schema::Move::Up => Move::Up,
                    gym::schema::Move::Down => Move::Down,
                    gym::schema::Move::Left => Move::Left,
                    gym::schema::Move::Right => Move::Right,
                };
                let (_, n) =
                    c.simulate_single_action(&[(ids["focal"], cm), (ids["other"], Move::Right)]);
                let t = step(&r, mv, gym::schema::Move::Right);
                for id in ["focal", "other"] {
                    let rs = t.state.snakes.iter().find(|s| s.id == id);
                    assert_eq!(
                        n.get_health_i64(&ids[id]),
                        rs.map_or(0, |s| s.health as i64)
                    );
                    if let Some(s) = rs {
                        let h = n.get_head_as_position(&ids[id]);
                        assert_eq!((h.x, h.y), (s.head.x, s.head.y));
                        assert_eq!(n.get_length(&ids[id]) as usize, s.length);
                    }
                }
            }
        }
    }
}

#[test]
fn other_body_collision_and_growth_blocked_next_tail() {
    let r = Request::new(
        vec![
            snake("focal", &[(4, 4), (4, 3), (3, 3)], 50),
            snake("other", &[(6, 5), (5, 5), (5, 4), (6, 4)], 50),
        ],
        vec![],
        "focal",
    );
    validate(&r).unwrap();
    assert!(
        step(&r, Move::Right, Move::Up)
            .eliminated
            .iter()
            .any(|e| e.snake.id == "focal" && e.cause == "body collision")
    );
    for growth in [false, true] {
        let r = Request::new(
            vec![
                snake("focal", &[(2, 2), (2, 1), (1, 1), (1, 2)], 50),
                snake("other", &[(0, 0), (0, 1), (0, 2)], 50),
            ],
            if growth { vec![p(2, 3)] } else { vec![] },
            "focal",
        );
        validate(&r).unwrap();
        let first = step(&r, Move::Up, Move::Right);
        let second = advance(
            &first.state,
            &BTreeMap::from([("focal".into(), Move::Up), ("other".into(), Move::Up)]),
            "focal",
            &Objective::Survive,
        );
        assert_eq!(
            second.eliminated.iter().any(|e| e.snake.id == "other"),
            growth,
            "growth delays release of the new tail by one turn"
        );
    }
}
