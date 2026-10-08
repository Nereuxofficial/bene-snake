use gym::{oracle::*, schema::*};
use std::time::Duration;
fn r(h: i32) -> Request {
    Request::new(
        vec![
            Snake::new(
                "f",
                vec![Point::new(0, 0), Point::new(0, 1), Point::new(1, 1)],
                h,
            ),
            Snake::new(
                "o",
                vec![Point::new(8, 8), Point::new(8, 7), Point::new(8, 6)],
                80,
            ),
        ],
        vec![],
        "f",
    )
}
fn limits(n: usize) -> Limits {
    Limits {
        nodes: n,
        certificate: n,
        wall: Duration::from_secs(10),
    }
}
#[test]
fn unique_escape_and_multiple_successes() {
    let s = solve(&r(80), &Objective::Survive, 1, limits(10000)).unwrap();
    assert_eq!(s.labels.successes(), vec![Move::Right]);
    assert_eq!(verify(&s.certificate.unwrap(), &r(80)).unwrap(), s.labels);
    let mut r = r(80);
    r.you = Snake::new(
        "f",
        vec![Point::new(3, 3), Point::new(3, 2), Point::new(2, 2)],
        80,
    );
    r.board.snakes[0] = r.you.clone();
    let s = solve(&r, &Objective::Survive, 2, limits(100000)).unwrap();
    assert_eq!(s.labels.successes().len(), 3);
    verify(&s.certificate.unwrap(), &r).unwrap();
}
#[test]
fn cutoffs_never_manufacture_labels() {
    for n in [0, 1, 2, 8] {
        let s = solve(&r(80), &Objective::Survive, 4, limits(n)).unwrap();
        assert!(s.labels.array().contains(&Label::Unknown));
        assert!(s.certificate.is_none());
    }
}
#[test]
fn horizon_objective_and_starvation_failure() {
    let s = solve(&r(1), &Objective::Survive, 1, limits(10000)).unwrap();
    assert!(s.labels.array().iter().all(|l| *l == Label::Failure));
    let s = solve(&r(80), &Objective::SoleSurvivor, 1, limits(10000)).unwrap();
    assert!(s.labels.array().iter().all(|l| *l == Label::Failure));
    assert!(!s.labels.accepted());
}
#[test]
fn adversarial_head_response() {
    let r = Request::new(
        vec![
            Snake::new(
                "f",
                vec![Point::new(3, 3), Point::new(2, 3), Point::new(1, 3)],
                70,
            ),
            Snake::new(
                "o",
                vec![Point::new(5, 3), Point::new(6, 3), Point::new(7, 3)],
                70,
            ),
        ],
        vec![Point::new(4, 3)],
        "f",
    );
    let s = solve(&r, &Objective::Survive, 1, limits(10000)).unwrap();
    assert_eq!(s.labels.right, Label::Failure);
    verify(&s.certificate.unwrap(), &r).unwrap();
}
#[test]
fn tampering_and_missing_universal_branches_rejected() {
    let r = r(80);
    let c = solve(&r, &Objective::Survive, 2, limits(100000))
        .unwrap()
        .certificate
        .unwrap();
    let mut bad = c.clone();
    bad.nodes[bad.roots[3]].actions[0].branches.pop();
    assert!(verify(&bad, &r).is_err());
    let mut bad = c.clone();
    bad.nodes[bad.roots[3]].actions[0].branches[0]
        .transition
        .state
        .eaten = true;
    assert!(verify(&bad, &r).is_err());
    let mut bad = c.clone();
    bad.nodes[bad.roots[3]].label = Label::Failure;
    assert!(verify(&bad, &r).is_err());
    let mut bad = c;
    bad.nodes[bad.roots[3]].actions[0].branches[0].child = usize::MAX;
    assert!(verify(&bad, &r).is_err());
}

#[test]
fn strategy_can_depend_on_observed_next_state() {
    let r = Request::new(
        vec![
            Snake::new(
                "f",
                vec![Point::new(4, 4), Point::new(4, 3), Point::new(3, 3)],
                50,
            ),
            Snake::new(
                "o",
                vec![Point::new(4, 7), Point::new(4, 8), Point::new(5, 8)],
                50,
            ),
        ],
        vec![],
        "f",
    );
    let s = solve(&r, &Objective::Survive, 2, limits(100000)).unwrap();
    assert_eq!(s.labels.up, Label::Success);
    let c = s.certificate.unwrap();
    verify(&c, &r).unwrap();
    let root = &c.nodes[c.roots[Move::Up.index()]];
    let choices = root.actions[0]
        .branches
        .iter()
        .filter_map(|b| c.nodes[b.child].actions.first().map(|a| a.chosen))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(choices.contains(&Move::Up));
    assert!(
        choices.len() > 1,
        "next policy must react to the opponent's observed head"
    );
}

#[test]
fn four_snake_coalition_keeps_every_cardinal_combination() {
    let mut r = r(80);
    r.board.snakes.extend([
        Snake::new(
            "third",
            vec![Point::new(5, 9), Point::new(5, 8), Point::new(5, 7)],
            50,
        ),
        Snake::new(
            "fourth",
            vec![Point::new(9, 4), Point::new(9, 3), Point::new(9, 2)],
            50,
        ),
    ]);
    let state = gym::rules::State::from_request(&r);
    assert_eq!(
        gym::rules::responses(&state, &r.you.id, Move::Right).len(),
        64
    );
    let sol = solve(&r, &Objective::Survive, 1, limits(100000)).unwrap();
    let c = sol.certificate.unwrap();
    assert_eq!(c.nodes[c.roots[3]].actions[0].branches.len(), 64);
    verify(&c, &r).unwrap();
}
