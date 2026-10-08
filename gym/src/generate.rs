//! Seeded family construction followed by exact certification. Constructors never
//! supply labels; all accepted directions are proved and independently verified.
use crate::{
    artifact::*,
    oracle::{self, Certificate, Limits},
    rules,
    schema::*,
};
use anyhow::{Result, bail, ensure};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub fn suite(name: &str) -> Result<Suite> {
    let s: Suite = if name == "hard-v1" || name == "hard-confirmation-v1" {
        let mut s: Suite = serde_json::from_str(include_str!("../suites/hard-v1.json"))?;
        if name == "hard-confirmation-v1" {
            s.name = "hard-v1-confirmation".into();
        }
        s
    } else if name == "balanced-v1" || name == "screening-v1" || name == "confirmation-v1" {
        let mut s: Suite = serde_json::from_str(include_str!("../suites/balanced-v1.json"))?;
        if name == "confirmation-v1" {
            s.name = "balanced-v1-confirmation".into();
        }
        s
    } else {
        read_json(Path::new(name))?
    };
    ensure!(
        s.schema == SCHEMA && !s.families.is_empty(),
        "unsupported/empty suite"
    );
    ensure!(
        s.families.iter().collect::<HashSet<_>>().len() == s.families.len(),
        "duplicate families"
    );
    ensure!(
        s.threshold >= 0.0 && s.max_trivial_share >= 0.0 && s.max_trivial_share <= 1.0,
        "invalid suite settings"
    );
    Ok(s)
}
fn p(x: i32, y: i32) -> Point {
    Point::new(x, y)
}
fn snake(id: &str, b: Vec<Point>, h: i32) -> Snake {
    Snake::new(id, b, h)
}
/// Valid symmetry transformation; all resulting certificates are regenerated and verified.
pub fn transform(r: &Request, t: usize) -> Request {
    let mut out = r.clone();
    let point = |mut q: Point| {
        if t >= 4 {
            q.x = 10 - q.x;
        }
        for _ in 0..t % 4 {
            q = p(10 - q.y, q.x);
        }
        q
    };
    for s in &mut out.board.snakes {
        s.head = point(s.head);
        s.body = s.body.iter().copied().map(point).collect();
    }
    out.board.food = out.board.food.iter().copied().map(point).collect();
    out.you = out
        .board
        .snakes
        .iter()
        .find(|s| s.id == r.you.id)
        .unwrap()
        .clone();
    out
}
/// IDs/names and symmetries are canonicalized. Full body geometry and health remain.
pub fn canonical(r: &Request, o: &Objective, h: u32) -> Result<String> {
    let mut keys = vec![];
    for t in 0..8 {
        let mut v = transform(r, t);
        let mut snakes = v
            .board
            .snakes
            .iter()
            .map(|s| (s.id == v.you.id, s.health, s.body.clone()))
            .collect::<Vec<_>>();
        snakes.sort();
        v.board.food.sort();
        let objective = match o {
            Objective::EatAndSurvive { food } => {
                let fr = Request::new(r.board.snakes.clone(), food.clone(), &r.you.id);
                let mut f = transform(&fr, t).board.food;
                f.sort();
                Objective::EatAndSurvive { food: f }
            }
            _ => o.clone(),
        };
        keys.push(serde_json::to_vec(&(snakes, &v.board.food, objective, h))?);
    }
    keys.sort();
    Ok(hash(&keys[0]))
}
pub fn construct(
    family: &str,
    seed: u64,
    confirmation: bool,
) -> Result<(Request, Objective, u32, BTreeMap<String, i64>)> {
    if crate::hard::FAMILIES.contains(&family) {
        return crate::hard::construct(family, seed, confirmation);
    }
    let mut rng = Rng(seed);
    let x = if confirmation {
        5 + rng.range(2) as i32
    } else {
        2 + rng.range(3) as i32
    };
    let y = 2 + rng.range(5) as i32;
    let enemy_h = if confirmation {
        90 + rng.range(10) as i32
    } else {
        30 + rng.range(60) as i32
    };
    let enemy = snake("opponent-z", vec![p(9, 9), p(9, 8), p(9, 7)], enemy_h);
    let mut params = BTreeMap::from([
        ("x".into(), x as i64),
        ("y".into(), y as i64),
        ("opponent_health".into(), enemy_h as i64),
    ]);
    let (snakes, food, objective, h) = match family {
        "unique_escape" => {
            let len = 3 + rng.range(5);
            let mut body = vec![p(0, 0), p(0, 1)];
            for i in 0..len - 2 {
                body.push(p(i as i32 + 1, 1));
            }
            (
                vec![
                    snake(
                        "focal-q",
                        body,
                        if confirmation {
                            10 + rng.range(30) as i32
                        } else {
                            40 + rng.range(60) as i32
                        },
                    ),
                    enemy,
                ],
                vec![],
                Objective::Survive,
                2,
            )
        }
        "head_contest" => {
            let len = 3 + rng.range(3);
            let mut a = vec![p(3, y), p(2, y), p(1, y)];
            if len > 3 {
                a.push(p(0, y));
            }
            let mut b = vec![p(5, y), p(6, y), p(7, y)];
            for i in 0..rng.range(3) {
                b.push(p(8 + i as i32, y));
            }
            params.insert("length_margin".into(), a.len() as i64 - b.len() as i64);
            (
                vec![snake("focal-q", a, 3), snake("opponent-z", b, enemy_h)],
                vec![p(4, y), p(3, y + 1)],
                Objective::Survive,
                3,
            )
        }
        "tail_growth" => {
            let stacked = rng.range(2) == 1;
            let mut body = vec![p(x, y), p(x, y - 1), p(x - 1, y - 1), p(x - 1, y)];
            if stacked {
                body.push(p(x - 1, y));
            }
            params.insert("tail_stacked".into(), stacked as i64);
            (
                vec![snake("focal-q", body, 3), enemy],
                vec![p(x - 2, y), p(x + 2, y)],
                Objective::Survive,
                3,
            )
        }
        "food_access" => {
            let distance = if rng.range(2) == 0 { 1 } else { 3 };
            let food = vec![p(x + distance, y), p(x, y - 2)];
            params.insert("food_distance".into(), distance as i64);
            (
                vec![
                    snake(
                        "focal-q",
                        vec![p(x, y), p(x, y - 1), p(x - 1, y - 1), p(x - 1, y - 2)],
                        distance + 1,
                    ),
                    enemy,
                ],
                food.clone(),
                Objective::EatAndSurvive { food },
                (distance + 1) as u32,
            )
        }
        "delayed_trap" => {
            let length = if confirmation {
                7
            } else {
                5 + rng.range(2) as i32
            };
            let yy = 3 + rng.range(3) as i32;
            let mut body = vec![p(length, yy), p(length, yy - 1)];
            for xx in (0..length).rev() {
                body.push(p(xx, yy - 1));
            }
            body.push(p(0, yy));
            body.push(p(0, yy + 1));
            for xx in 1..=length + 2 {
                body.push(p(xx, yy + 1));
            }
            body.extend([
                p(length + 2, yy + 2),
                p(length + 2, yy + 3),
                p(length + 1, yy + 3),
                p(length, yy + 3),
            ]);
            params.insert("corridor_length".into(), length as i64);
            params.insert("corridor_y".into(), yy as i64);
            let rival = snake("opponent-z", vec![p(9, 1), p(9, 0), p(10, 0)], enemy_h);
            (
                vec![snake("focal-q", body, 60 + rng.range(40) as i32), rival],
                vec![],
                Objective::Survive,
                length as u32,
            )
        }
        "forced_win" => {
            let len = 3 + rng.range(3);
            let mut b = vec![p(x, y), p(x, y - 1), p(x - 1, y - 1)];
            for i in 0..len - 3 {
                b.push(p(x - 1, y - 2 - i as i32));
            }
            let rival_y = if confirmation {
                if rng.range(2) == 0 { 8 } else { 10 }
            } else {
                9
            };
            let rival_x = if confirmation {
                9
            } else {
                8 + (seed & 1) as i32
            };
            params.insert("rival_x".into(), rival_x as i64);
            params.insert("rival_y".into(), rival_y as i64);
            params.insert("opponent_health".into(), 2);
            let rival = snake(
                "opponent-z",
                vec![
                    p(rival_x, rival_y),
                    p(rival_x, rival_y - 1),
                    p(rival_x, rival_y - 2),
                ],
                2,
            );
            (
                vec![snake("focal-q", b, 2), rival],
                vec![p(x + 2, y)],
                Objective::SoleSurvivor,
                2,
            )
        }
        _ => bail!("unknown family {family}"),
    };
    let mut request = Request::new(snakes, food, "focal-q");
    let t = rng.range(8);
    request = transform(&request, t);
    let objective = if let Objective::EatAndSurvive { .. } = objective {
        Objective::EatAndSurvive {
            food: request.board.food.clone(),
        }
    } else {
        objective
    };
    params.insert("symmetry".into(), t as i64);
    rules::validate(&request)?;
    Ok((request, objective, h, params))
}
fn immediate(r: &Request, o: &Objective, m: Move) -> bool {
    let s = rules::State::from_request(r);
    rules::responses(&s, &r.you.id, m).iter().all(|a| {
        !rules::advance(&s, a, &r.you.id, o)
            .state
            .snakes
            .iter()
            .any(|s| s.id == r.you.id)
    })
}
fn acceptance(f: &str, r: &Request, o: &Objective, l: &Labels, c: &Certificate) -> Result<u32> {
    ensure!(l.accepted(), "unresolved or nondiscriminating labels");
    let delay = Move::ALL
        .iter()
        .filter(|m| l.get(**m) == Label::Failure)
        .map(|m| oracle::failure_delay(c, c.roots[m.index()]))
        .max()
        .unwrap_or(0);
    match f {
        f if crate::hard::FAMILIES.contains(&f) => crate::hard::acceptance(r, l, c)?,
        "unique_escape" | "forced_win" => {
            ensure!(l.successes().len() == 1, "unique objective not established")
        }
        "delayed_trap" => ensure!(
            delay > 4
                && Move::ALL
                    .iter()
                    .any(|m| l.get(*m) == Label::Failure && !immediate(r, o, *m)),
            "no demonstrated delayed failure beyond four"
        ),
        "tail_growth" => {
            let own = &r.you;
            ensure!(
                Move::ALL.iter().any(|m| {
                    let q = m.step(own.head);
                    own.body.last() == Some(&q)
                        && (l.get(*m) == Label::Success || own.body[own.body.len() - 2] == q)
                }),
                "no tail-dependent root decision"
            );
        }
        _ => {}
    }
    Ok(delay)
}

/// Drop speculative solver nodes outside the final strategy/counterstrategy DAG.
/// Reachable universal branches are retained; verification still checks the full
/// resulting proof. Stable original node order makes remapping deterministic.
pub fn compact_certificate(mut c: Certificate) -> Result<Certificate> {
    let mut reachable = std::collections::BTreeSet::new();
    let mut stack = c.roots.to_vec();
    while let Some(id) = stack.pop() {
        ensure!(id < c.nodes.len(), "certificate reference outside DAG");
        if reachable.insert(id) {
            for action in &c.nodes[id].actions {
                stack.extend(action.branches.iter().map(|b| b.child));
            }
        }
    }
    let mut remap = vec![usize::MAX; c.nodes.len()];
    for (new, old) in reachable.iter().enumerate() {
        remap[*old] = new;
    }
    c.nodes = c
        .nodes
        .into_iter()
        .enumerate()
        .filter_map(|(id, node)| reachable.contains(&id).then_some(node))
        .collect();
    for root in &mut c.roots {
        *root = remap[*root];
    }
    for node in &mut c.nodes {
        for action in &mut node.actions {
            for branch in &mut action.branches {
                branch.child = remap[branch.child];
            }
        }
    }
    Ok(c)
}
pub fn generate(out: &Path, suite: Suite, count: usize, seed: u64) -> Result<Generation> {
    ensure!(
        count > 0 && count.is_multiple_of(suite.families.len()),
        "cases must be positive and divisible by {} families",
        suite.families.len()
    );
    ensure!(
        !out.join("generation.json").exists() && !out.join("corpus.jsonl").exists(),
        "output corpus already exists"
    );
    fs::create_dir_all(out.join("certificates"))?;
    let start = Instant::now();
    let mut cases = vec![];
    let mut families = BTreeMap::new();
    let mut seen = HashSet::new();
    let quota = count / suite.families.len();
    let mut trivial = 0;
    for family in &suite.families {
        let ft = Instant::now();
        let mut g = FamilyGeneration {
            accepted: 0,
            attempts: 0,
            rejections: BTreeMap::new(),
            nodes: 0,
            elapsed_ms: 0,
            horizons: BTreeMap::new(),
        };
        for index in 0..quota {
            for attempt in 0..suite.max_attempts_per_case {
                g.attempts += 1;
                let attempt_seed = stream(seed, family, index, attempt);
                let result = (|| -> Result<(Case, Certificate)> {
                    let (r, o, h, parameters) =
                        construct(family, attempt_seed, suite.name.contains("confirmation"))?;
                    let cluster = canonical(&r, &o, h)?;
                    ensure!(!seen.contains(&cluster), "canonical duplicate");
                    let sol = oracle::solve(
                        &r,
                        &o,
                        h,
                        Limits {
                            nodes: suite.node_limit,
                            certificate: suite.certificate_limit,
                            wall: Duration::from_millis(suite.wall_ms),
                        },
                    )?;
                    g.nodes += sol.explored;
                    ensure!(
                        sol.labels.accepted(),
                        "{}",
                        sol.cutoff.unwrap_or("nondiscriminating labels".into())
                    );
                    let cert = if crate::hard::FAMILIES.contains(&family.as_str()) {
                        compact_certificate(sol.certificate.unwrap())?
                    } else {
                        sol.certificate.unwrap()
                    };
                    ensure!(
                        oracle::verify(&cert, &r)? == sol.labels,
                        "verifier mismatch"
                    );
                    let delay = acceptance(family, &r, &o, &sol.labels, &cert)?;
                    let is_trivial = Move::ALL
                        .iter()
                        .filter(|m| sol.labels.get(**m) == Label::Failure)
                        .all(|m| immediate(&r, &o, *m));
                    ensure!(
                        !is_trivial
                            || (trivial + 1) as f64 <= count as f64 * suite.max_trivial_share,
                        "trivial share limit"
                    );
                    if is_trivial {
                        trivial += 1;
                    }
                    let certificate_hash = value_hash(&cert)?;
                    let id = format!("{}-{}", family, &cluster[..16]);
                    Ok((
                        Case {
                            schema: SCHEMA,
                            id,
                            cluster,
                            family: family.clone(),
                            family_version: 1,
                            seed: attempt_seed,
                            parameters,
                            request: r,
                            objective: o,
                            horizon: h,
                            rules_hash: rules_hash(),
                            oracle_hash: oracle_hash(),
                            labels: sol.labels,
                            certificate_hash,
                            provenance: "synthetic; opening reachability unverified".into(),
                            nodes: sol.explored,
                            minimum_failure_delay: delay,
                        },
                        cert,
                    ))
                })();
                match result {
                    Ok((case, cert)) => {
                        seen.insert(case.cluster.clone());
                        let path = out
                            .join("certificates")
                            .join(format!("{}.json", case.certificate_hash));
                        if crate::hard::FAMILIES.contains(&family.as_str()) {
                            atomic(&path, &serde_json::to_vec(&cert)?)?;
                        } else {
                            write_json(&path, &cert)?;
                        }
                        *g.horizons.entry(case.horizon).or_default() += 1;
                        cases.push(case);
                        g.accepted += 1;
                        break;
                    }
                    Err(e) => {
                        *g.rejections.entry(e.to_string()).or_default() += 1;
                    }
                }
            }
        }
        g.elapsed_ms = ft.elapsed().as_millis() as u64;
        eprintln!(
            "generate {family}: {}/{} accepted, {} attempts, {} ms, {} nodes",
            g.accepted, quota, g.attempts, g.elapsed_ms, g.nodes
        );
        families.insert(family.clone(), g);
    }
    let mut bytes = vec![];
    for c in &cases {
        serde_json::to_writer(&mut bytes, c)?;
        bytes.push(b'\n');
    }
    atomic(&out.join("corpus.jsonl"), &bytes)?;
    let g = Generation {
        schema: SCHEMA,
        seed,
        requested: count,
        complete: cases.len() == count,
        suite,
        generator_hash: generator_hash(),
        rules_hash: rules_hash(),
        oracle_hash: oracle_hash(),
        corpus_hash: hash(&bytes),
        families,
        elapsed_ms: start.elapsed().as_millis() as u64,
    };
    write_json(&out.join("generation.json"), &g)?;
    Ok(g)
}
pub fn load(out: &Path) -> Result<(Generation, Vec<Case>)> {
    let g: Generation = read_json(&out.join("generation.json"))?;
    ensure!(
        g.schema == SCHEMA
            && g.suite.schema == SCHEMA
            && g.requested > 0
            && !g.suite.families.is_empty()
            && g.rules_hash == rules_hash()
            && g.oracle_hash == oracle_hash(),
        "incompatible corpus versions/implementation hashes"
    );
    ensure!(
        g.corpus_hash == file_hash(&out.join("corpus.jsonl"))?,
        "corpus hash mismatch"
    );
    let cases: Vec<Case> = read_jsonl(&out.join("corpus.jsonl"), false)?;
    let mut ids = HashSet::new();
    let mut clusters = HashSet::new();
    for c in &cases {
        ensure!(
            c.schema == SCHEMA
                && c.family_version == 1
                && ids.insert(&c.id)
                && clusters.insert(&c.cluster),
            "duplicate/incompatible case"
        );
        ensure!(
            c.rules_hash == g.rules_hash
                && c.oracle_hash == g.oracle_hash
                && g.suite.families.contains(&c.family),
            "case contract mismatch"
        );
        ensure!(
            c.certificate_hash.len() == 64
                && c.certificate_hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid certificate identity"
        );
        let cert: Certificate = read_json(
            &out.join("certificates")
                .join(format!("{}.json", c.certificate_hash)),
        )?;
        ensure!(
            value_hash(&cert)? == c.certificate_hash,
            "certificate hash mismatch"
        );
        ensure!(
            cert.objective == c.objective
                && cert.horizon == c.horizon
                && oracle::verify(&cert, &c.request)? == c.labels,
            "case labels/objective mismatch"
        );
        acceptance(&c.family, &c.request, &c.objective, &c.labels, &cert)?;
        ensure!(
            canonical(&c.request, &c.objective, c.horizon)? == c.cluster,
            "cluster mismatch"
        );
    }
    ensure!(
        !g.complete || cases.len() == g.requested,
        "incomplete corpus marked complete"
    );
    Ok((g, cases))
}
