//! Readable wire-state standard rules: movement, health, feeding, preliminary
//! elimination, simultaneous collisions. Food grows the *new* tail (duplicates it).
//! Specification: BattlesnakeOfficial/rules v1.2.3 standard.go.
use crate::schema::*;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn validate(r: &Request) -> Result<()> {
    ensure!(
        r.game.ruleset.name == "standard" && r.game.map == "standard",
        "unsupported rules/map"
    );
    ensure!(
        r.game.ruleset.settings.food_spawn_chance == 0 && r.game.ruleset.settings.minimum_food == 0,
        "future food spawning unsupported"
    );
    ensure!(
        r.board.width == 11 && r.board.height == 11 && r.board.hazards.is_empty(),
        "unsupported dimensions/hazards"
    );
    ensure!(
        (2..=4).contains(&r.board.snakes.len()),
        "need 2-4 live snakes"
    );
    let mut ids = BTreeSet::new();
    let mut occupied = BTreeSet::new();
    for s in &r.board.snakes {
        ensure!(
            !s.id.is_empty() && ids.insert(&s.id),
            "duplicate/empty snake ID"
        );
        ensure!(
            (1..=100).contains(&s.health) && (3..=121).contains(&s.body.len()),
            "invalid health/length"
        );
        ensure!(
            s.length == s.body.len() && s.body.first() == Some(&s.head),
            "inconsistent head/length"
        );
        let mut seen = BTreeSet::new();
        for (i, p) in s.body.iter().enumerate() {
            ensure!(p.inside(), "body outside board");
            if i > 0 {
                let q = s.body[i - 1];
                let d = (q.x - p.x).abs() + (q.y - p.y).abs();
                ensure!(d <= 1, "noncontiguous body");
            }
            if !seen.insert(*p) {
                ensure!(
                    s.body[i..].iter().all(|q| q == p)
                        && s.body[..i].iter().filter(|q| *q == p).count() <= 2,
                    "only tail stacks up to three allowed"
                );
            }
        }
        for p in seen {
            ensure!(occupied.insert(p), "overlapping snakes");
        }
    }
    ensure!(
        r.board.snakes.iter().find(|s| s.id == r.you.id) == Some(&r.you),
        "focal snake mismatch"
    );
    let mut food = BTreeSet::new();
    for p in &r.board.food {
        ensure!(
            p.inside() && !occupied.contains(p) && food.insert(*p),
            "invalid food/body overlap"
        );
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub snakes: Vec<Snake>,
    pub food: Vec<Point>,
    pub eaten: bool,
}
impl State {
    pub fn from_request(r: &Request) -> Self {
        let mut s = Self {
            snakes: r.board.snakes.clone(),
            food: r.board.food.clone(),
            eaten: false,
        };
        s.snakes.sort_by(|a, b| a.id.cmp(&b.id));
        s.food.sort();
        s
    }
    pub fn terminal(&self, focal: &str, o: &Objective, depth: u32) -> Option<(Label, String)> {
        if !self.snakes.iter().any(|s| s.id == focal) {
            return Some((
                Label::Failure,
                "focal eliminated (extinction also fails)".into(),
            ));
        }
        if self.snakes.len() == 1 {
            return Some((Label::Success, "focal is sole survivor".into()));
        }
        if depth == 0 {
            let success = match o {
                Objective::Survive => true,
                Objective::EatAndSurvive { .. } => self.eaten,
                Objective::SoleSurvivor => false,
            };
            return Some((
                if success {
                    Label::Success
                } else {
                    Label::Failure
                },
                "finite horizon objective".into(),
            ));
        }
        None
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub state: State,
    pub eliminated: Vec<Eliminated>,
    pub consumed: Vec<Point>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Eliminated {
    pub snake: Snake,
    pub cause: String,
}
/// All four cardinal responses, even reverse or fatal moves; no legality pruning.
pub fn responses(s: &State, focal: &str, m: Move) -> Vec<BTreeMap<String, Move>> {
    let mut out = vec![BTreeMap::from([(focal.into(), m)])];
    for snake in s.snakes.iter().filter(|s| s.id != focal) {
        out = out
            .into_iter()
            .flat_map(|a| {
                Move::ALL.map(|mv| {
                    let mut b = a.clone();
                    b.insert(snake.id.clone(), mv);
                    b
                })
            })
            .collect();
    }
    out
}
pub fn advance(
    s: &State,
    moves: &BTreeMap<String, Move>,
    focal: &str,
    o: &Objective,
) -> Transition {
    let mut next = s.clone();
    let mut consumed = BTreeSet::new();
    for snake in &mut next.snakes {
        let head = moves[&snake.id].step(snake.head);
        snake.body.insert(0, head);
        snake.body.pop();
        snake.head = head;
        snake.health -= 1;
        if s.food.contains(&head) {
            snake.body.push(*snake.body.last().unwrap());
            snake.health = 100;
            consumed.insert(head);
            if snake.id == focal
                && let Objective::EatAndSurvive { food } = o
                && food.contains(&head)
            {
                next.eaten = true;
            }
        }
        snake.length = snake.body.len();
    }
    next.food.retain(|p| !consumed.contains(p));
    let mut deaths = BTreeMap::new();
    // Starving/out-of-bounds snakes are removed before collision checks, as in standard.go.
    for snake in &next.snakes {
        if snake.health <= 0 {
            deaths.insert(snake.id.clone(), "starvation".to_string());
        } else if !snake.head.inside() {
            deaths.insert(snake.id.clone(), "wall".to_string());
        }
    }
    let active: Vec<_> = next
        .snakes
        .iter()
        .filter(|s| !deaths.contains_key(&s.id))
        .collect();
    let mut collision = BTreeMap::new();
    for snake in &active {
        let cause = if snake.body[1..].contains(&snake.head) {
            Some("self collision")
        } else if active
            .iter()
            .any(|s| s.id != snake.id && s.body[1..].contains(&snake.head))
        {
            Some("body collision")
        } else if active
            .iter()
            .any(|s| s.id != snake.id && s.head == snake.head && s.length >= snake.length)
        {
            Some("head contest")
        } else {
            None
        };
        if let Some(c) = cause {
            collision.insert(snake.id.clone(), c.to_string());
        }
    }
    deaths.extend(collision);
    let eliminated = next
        .snakes
        .iter()
        .filter_map(|s| {
            deaths.get(&s.id).map(|c| Eliminated {
                snake: s.clone(),
                cause: c.clone(),
            })
        })
        .collect();
    next.snakes.retain(|s| !deaths.contains_key(&s.id));
    Transition {
        state: next,
        eliminated,
        consumed: consumed.into_iter().collect(),
    }
}
