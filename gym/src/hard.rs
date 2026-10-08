//! Candidate-independent dense geometry proposals for the challenge distribution.
//! Random walks propose bodies, not labels. All accepted cases still require the
//! exact oracle and full DAG verification in `generate`.
use crate::{artifact::Rng, generate::transform, oracle, rules, schema::*};
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub const FAMILIES: [&str; 3] = ["crowded_duel", "coalition_escape", "starvation_detour"];

fn distance(a: Point, b: Point) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn walk(
    rng: &mut Rng,
    head: Point,
    length: usize,
    occupied: &BTreeSet<Point>,
) -> Result<Vec<Point>> {
    ensure!(!occupied.contains(&head), "occupied proposed head");
    // Bounded restarts avoid accepting disconnected or overlapping bodies.
    for _ in 0..16 {
        let mut body = vec![head];
        let mut seen = occupied.clone();
        seen.insert(head);
        while body.len() < length {
            let next: Vec<_> = Move::ALL
                .iter()
                .map(|m| m.step(*body.last().unwrap()))
                .filter(|q| q.inside() && !seen.contains(q))
                .collect();
            if next.is_empty() {
                break;
            }
            let q = next[rng.range(next.len())];
            seen.insert(q);
            body.push(q);
        }
        if body.len() == length {
            return Ok(body);
        }
    }
    bail!("bounded body walk exhausted")
}

pub fn construct(
    family: &str,
    seed: u64,
    confirmation: bool,
) -> Result<(Request, Objective, u32, BTreeMap<String, i64>)> {
    let mut rng = Rng(seed);
    let head = Point::new(3 + rng.range(5) as i32, 3 + rng.range(5) as i32);
    let (length, health, horizon, rivals) = match family {
        "crowded_duel" => (
            16 + rng.range(17),
            20 + rng.range(60) as i32,
            if confirmation {
                9
            } else {
                7 + rng.range(2) as u32
            },
            1,
        ),
        "coalition_escape" => (
            8 + rng.range(13),
            12 + rng.range(60) as i32,
            if confirmation { 5 } else { 4 },
            2,
        ),
        "starvation_detour" => {
            let health = if confirmation {
                9 + rng.range(2)
            } else {
                6 + rng.range(3)
            };
            (16 + rng.range(17), health as i32, health as u32 + 1, 1)
        }
        _ => bail!("unknown challenge family"),
    };
    let mut occupied = BTreeSet::new();
    let body = walk(&mut rng, head, length, &occupied)?;
    occupied.extend(body.iter().copied());
    let mut snakes = vec![Snake::new("focal-q", body, health)];
    for index in 0..rivals {
        let heads: Vec<_> = (0..11)
            .flat_map(|x| (0..11).map(move |y| Point::new(x, y)))
            .filter(|q| {
                !occupied.contains(q)
                    && if family == "starvation_detour" {
                        distance(*q, head) >= 6
                    } else {
                        (2..=5).contains(&distance(*q, head))
                    }
            })
            .collect();
        ensure!(!heads.is_empty(), "no rival head available");
        let rival_head = heads[rng.range(heads.len())];
        let rival_length = if rivals == 2 {
            8 + rng.range(9)
        } else {
            12 + rng.range(17)
        };
        let body = walk(&mut rng, rival_head, rival_length, &occupied)?;
        occupied.extend(body.iter().copied());
        snakes.push(Snake::new(
            &format!("rival-{index}"),
            body,
            50 + rng.range(51) as i32,
        ));
    }
    let mut food = vec![];
    let food_count = if family == "starvation_detour" {
        2
    } else {
        rng.range(3)
    };
    for _ in 0..food_count {
        let free: Vec<_> = (0..11)
            .flat_map(|x| (0..11).map(move |y| Point::new(x, y)))
            .filter(|q| {
                !occupied.contains(q)
                    && !food.contains(q)
                    && (family != "starvation_detour" || (2..health).contains(&distance(*q, head)))
            })
            .collect();
        ensure!(!free.is_empty(), "no food proposal available");
        food.push(free[rng.range(free.len())]);
    }
    let symmetry = rng.range(8);
    let r = transform(&Request::new(snakes, food, "focal-q"), symmetry);
    rules::validate(&r)?;
    let params = BTreeMap::from([
        ("focal_length".into(), length as i64),
        ("focal_health".into(), health as i64),
        ("rivals".into(), rivals as i64),
        ("horizon".into(), horizon as i64),
        ("symmetry".into(), symmetry as i64),
        ("confirmation_range".into(), confirmation as i64),
    ]);
    Ok((r, Objective::Survive, horizon, params))
}

/// Two robustly safe first moves and one proven later failure prevent immediate
/// wall/reverse/head threats alone from qualifying a challenge position.
pub fn acceptance(r: &Request, labels: &Labels, certificate: &oracle::Certificate) -> Result<()> {
    let state = rules::State::from_request(r);
    let safe = |m| {
        rules::responses(&state, &r.you.id, m).iter().all(|a| {
            rules::advance(&state, a, &r.you.id, &Objective::Survive)
                .state
                .snakes
                .iter()
                .any(|s| s.id == r.you.id)
        })
    };
    ensure!(
        Move::ALL.iter().filter(|m| safe(**m)).count() >= 2,
        "fewer than two robustly safe first moves"
    );
    ensure!(
        Move::ALL.iter().any(|m| labels.get(*m) == Label::Failure
            && safe(*m)
            && oracle::failure_delay(certificate, certificate.roots[m.index()]) >= 4),
        "no certified later failure after a robustly safe first move"
    );
    Ok(())
}
