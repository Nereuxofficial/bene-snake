use gym::{artifact::*, generate::*, oracle::*, schema::*};
use std::{path::PathBuf, time::Duration};
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gym-{name}-{}-{}", std::process::id(), timestamp()))
}
#[test]
fn all_families_verified_deterministic_and_fresh_geometry() {
    let s = suite("balanced-v1").unwrap();
    let a = temp("gen-a");
    let b = temp("gen-b");
    let c = temp("gen-c");
    let ga = generate(&a, s.clone(), 12, 1234).unwrap();
    assert!(ga.complete);
    assert!(generate(&b, s.clone(), 12, 1234).unwrap().complete);
    assert!(generate(&c, s, 12, 5678).unwrap().complete);
    assert_eq!(
        std::fs::read(a.join("corpus.jsonl")).unwrap(),
        std::fs::read(b.join("corpus.jsonl")).unwrap()
    );
    let (_, cases) = load(&a).unwrap();
    let (_, fresh) = load(&c).unwrap();
    assert_ne!(
        cases.iter().map(|c| &c.cluster).collect::<Vec<_>>(),
        fresh.iter().map(|c| &c.cluster).collect::<Vec<_>>()
    );
    for case in cases {
        let filename = format!("{}.json", case.certificate_hash);
        assert_eq!(
            std::fs::read(a.join("certificates").join(&filename)).unwrap(),
            std::fs::read(b.join("certificates").join(&filename)).unwrap()
        );
        if case.family == "delayed_trap" {
            assert!(case.minimum_failure_delay > 4);
        }
        if case.family == "forced_win" {
            assert_eq!(case.objective, Objective::SoleSurvivor);
            assert_eq!(case.labels.successes().len(), 1);
        }
    }
    for p in [a, b, c] {
        std::fs::remove_dir_all(p).unwrap();
    }
}
#[test]
fn symmetry_labels_and_canonical_dedup() {
    let (r, o, h, _) = construct("unique_escape", 13, false).unwrap();
    let key = canonical(&r, &o, h).unwrap();
    let original = solve(
        &r,
        &o,
        h,
        Limits {
            nodes: 100000,
            certificate: 100000,
            wall: Duration::from_secs(10),
        },
    )
    .unwrap();
    for t in 0..8 {
        let r2 = transform(&r, t);
        assert_eq!(canonical(&r2, &o, h).unwrap(), key);
        let s = solve(
            &r2,
            &o,
            h,
            Limits {
                nodes: 100000,
                certificate: 100000,
                wall: Duration::from_secs(10),
            },
        )
        .unwrap();
        let c = s.certificate.unwrap();
        verify(&c, &r2).unwrap();
        for m in Move::ALL {
            let to = m.step(r.you.head);
            let mut dummy = r.clone();
            dummy.you.head = to;
            dummy
                .board
                .snakes
                .iter_mut()
                .find(|s| s.id == dummy.you.id)
                .unwrap()
                .head = to;
            let transformed = transform(&dummy, t).you.head;
            let tm = Move::ALL
                .into_iter()
                .find(|mv| mv.step(r2.you.head) == transformed)
                .unwrap();
            assert_eq!(original.labels.get(m), s.labels.get(tm));
        }
    }
}
#[test]
fn quota_shortage_is_bounded_and_outside_scoring() {
    let out = temp("short");
    let mut s = suite("balanced-v1").unwrap();
    s.node_limit = 0;
    s.max_attempts_per_case = 2;
    let g = generate(&out, s, 6, 1).unwrap();
    assert!(!g.complete);
    assert!(
        g.families
            .values()
            .all(|f| f.accepted == 0 && f.attempts == 2)
    );
    assert!(load(&out).unwrap().1.is_empty());
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
fn reviewed_seed_examples_reconstruct_and_verify() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/reviewed-v1.json")).unwrap();
    for e in fixture["examples"].as_array().unwrap() {
        let family = e["family"].as_str().unwrap();
        let seed = e["seed"].as_u64().unwrap();
        let request: Request = serde_json::from_value(e["request"].clone()).unwrap();
        let objective: Objective = serde_json::from_value(e["objective"].clone()).unwrap();
        let labels: Labels = serde_json::from_value(e["labels"].clone()).unwrap();
        let h = e["horizon"].as_u64().unwrap() as u32;
        let (reconstructed, o, depth, _) = construct(family, seed, false).unwrap();
        assert_eq!(reconstructed, request);
        assert_eq!(o, objective);
        assert_eq!(depth, h);
        let sol = solve(
            &request,
            &objective,
            h,
            Limits {
                nodes: 120000,
                certificate: 60000,
                wall: Duration::from_secs(10),
            },
        )
        .unwrap();
        assert_eq!(sol.labels, labels);
        assert_eq!(verify(&sol.certificate.unwrap(), &request).unwrap(), labels);
    }
}

#[test]
fn default_confirmation_quota_has_enough_unique_geometry() {
    let out = temp("confirmation-default");
    let g = generate(&out, suite("confirmation-v1").unwrap(), 240, 20261013).unwrap();
    assert!(
        g.complete,
        "default confirmation quotas must fit constructor support"
    );
    let (_, cases) = load(&out).unwrap();
    assert_eq!(cases.len(), 240);
    for c in cases {
        if c.family == "forced_win" {
            assert!([8, 10].contains(&c.parameters["rival_y"]));
        }
        if c.family == "head_contest" {
            assert!(c.parameters["opponent_health"] >= 90);
        }
        if c.family == "delayed_trap" {
            assert_eq!(c.parameters["corridor_length"], 7);
        }
    }
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
fn challenge_corpus_is_certified_reproducible_and_has_delayed_choices() {
    let a = temp("hard-a");
    let b = temp("hard-b");
    let s = suite("hard-v1").unwrap();
    assert!(generate(&a, s.clone(), 3, 20261019).unwrap().complete);
    assert!(generate(&b, s, 3, 20261019).unwrap().complete);
    assert_eq!(
        std::fs::read(a.join("corpus.jsonl")).unwrap(),
        std::fs::read(b.join("corpus.jsonl")).unwrap()
    );
    let (_, cases) = load(&a).unwrap();
    for c in cases {
        let certificate: Certificate = read_json(
            &a.join("certificates")
                .join(format!("{}.json", c.certificate_hash)),
        )
        .unwrap();
        let state = gym::rules::State::from_request(&c.request);
        let safe = |m| {
            gym::rules::responses(&state, &c.request.you.id, m)
                .iter()
                .all(|a| {
                    gym::rules::advance(&state, a, &c.request.you.id, &c.objective)
                        .state
                        .snakes
                        .iter()
                        .any(|s| s.id == c.request.you.id)
                })
        };
        assert!(Move::ALL.iter().filter(|m| safe(**m)).count() >= 2);
        assert!(Move::ALL.iter().any(|m| safe(*m)
            && c.labels.get(*m) == Label::Failure
            && failure_delay(&certificate, certificate.roots[m.index()]) >= 4));
        assert_eq!(verify(&certificate, &c.request).unwrap(), c.labels);
        assert_eq!(c.objective, Objective::Survive);
        assert_eq!(
            c.request.board.snakes.len(),
            if c.family == "coalition_escape" { 3 } else { 2 }
        );
    }
    for p in [a, b] {
        std::fs::remove_dir_all(p).unwrap();
    }
}

#[test]
fn challenge_confirmation_uses_reserved_depth_and_health_ranges() {
    assert_ne!(
        suite("hard-v1").unwrap().name,
        suite("hard-confirmation-v1").unwrap().name
    );
    for family in ["crowded_duel", "coalition_escape", "starvation_detour"] {
        let examples: Vec<_> = (0..20)
            .filter_map(|seed| construct(family, seed, true).ok())
            .collect();
        assert!(!examples.is_empty());
        for (request, objective, horizon, parameters) in examples {
            gym::rules::validate(&request).unwrap();
            assert_eq!(objective, Objective::Survive);
            assert_eq!(parameters["confirmation_range"], 1);
            match family {
                "crowded_duel" => assert_eq!(horizon, 9),
                "coalition_escape" => assert_eq!(horizon, 5),
                _ => assert!((9..=10).contains(&request.you.health)),
            }
        }
    }
}

#[test]
fn proof_compaction_preserves_labels_universal_branches_and_playback() {
    let (r, o, h, _) = construct("tail_growth", 13, false).unwrap();
    let s = solve(
        &r,
        &o,
        h,
        Limits {
            nodes: 120000,
            certificate: 60000,
            wall: Duration::from_secs(10),
        },
    )
    .unwrap();
    let mut original = s.certificate.unwrap();
    // A deliberately unreferenced node exercises removal, including proofs
    // where the solver's original search produced no speculative orphan.
    original
        .nodes
        .push(original.nodes[original.roots[0]].clone());
    let compact = compact_certificate(original.clone()).unwrap();
    assert!(compact.nodes.len() < original.nodes.len());
    assert_eq!(verify(&compact, &r).unwrap(), s.labels);
    for m in Move::ALL {
        assert_eq!(
            serde_json::to_value(example(&original, m)).unwrap(),
            serde_json::to_value(example(&compact, m)).unwrap()
        );
    }
    let mut missing = compact;
    let success = missing.roots[s.labels.successes()[0].index()];
    missing.nodes[success].actions[0].branches.pop();
    assert!(verify(&missing, &r).is_err());
}
