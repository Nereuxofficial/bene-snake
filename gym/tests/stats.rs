use gym::{artifact::*, generate, schema::*, stats::*};
fn fixture() -> (Manifest, Vec<Case>) {
    let out = std::env::temp_dir().join(format!(
        "gym-stats-{}-{}-{}",
        std::process::id(),
        timestamp(),
        std::thread::current().name().unwrap_or("test")
    ));
    generate::generate(&out, generate::suite("balanced-v1").unwrap(), 6, 1234).unwrap();
    let (_, base) = generate::load(&out).unwrap();
    std::fs::remove_dir_all(out).unwrap();
    let mut cases = vec![];
    for (f, item) in base.iter().enumerate().take(2) {
        for i in 0..6 {
            let mut c = item.clone();
            c.id = format!("f{f}-i{i}");
            c.cluster = format!("f{f}-cluster{i}");
            cases.push(c);
        }
    }
    let candidate = Candidate {
        descriptor: "mock".into(),
        sha: None,
        source_hash: None,
        lock_hash: None,
        binary_hash: "bin".into(),
        binary: "bin".into(),
        toolchain: "mock".into(),
        flags: vec![],
        build_log: None,
        submodules: String::new(),
    };
    let mut suite = generate::suite("balanced-v1").unwrap();
    suite.families = base[..2].iter().map(|c| c.family.clone()).collect();
    suite.min_clusters_per_family = 5;
    let m = Manifest {
        schema: SCHEMA,
        run_id: "run".into(),
        created: String::new(),
        corpus_hash: String::new(),
        generation_hash: String::new(),
        harness_hash: String::new(),
        rules_hash: rules_hash(),
        oracle_hash: oracle_hash(),
        candidates: [candidate.clone(), candidate],
        repeats: 2,
        timeout_ms: 100,
        suite,
        machine: String::new(),
        mode: String::new(),
        environment: Default::default(),
        build_ms: 0,
    };
    (m, cases)
}
fn rows(
    m: &Manifest,
    cases: &[Case],
    scores: impl Fn(usize, usize, usize) -> bool,
) -> Vec<Attempt> {
    let mut rows = vec![];
    for (i, c) in cases.iter().enumerate() {
        for r in 0..m.repeats {
            for a in 0..2 {
                let chosen = if scores(i, r, a) {
                    c.labels.successes()[0]
                } else {
                    Move::ALL
                        .into_iter()
                        .find(|mv| c.labels.get(*mv) == Label::Failure)
                        .unwrap()
                };
                rows.push(Attempt {
                    schema: SCHEMA,
                    run_id: m.run_id.clone(),
                    case_id: c.id.clone(),
                    candidate: a,
                    repeat: r,
                    order: rows.len(),
                    request_hash: String::new(),
                    binary_hash: "bin".into(),
                    status: Status::Valid,
                    chosen: Some(chosen),
                    response: None,
                    latency_ms: 20.0,
                    startup_ms: 10.0,
                    deadline_ms: 100,
                    timestamp: String::new(),
                    log: String::new(),
                    detail: String::new(),
                });
            }
        }
    }
    rows
}
#[test]
fn exact_means_swap_negates_interval_and_repeats_do_not_inflate_n() {
    let (m, c) = fixture();
    let r = rows(&m, &c, |i, rep, a| {
        if a == 0 {
            i < 6 || rep == 0
        } else {
            i >= 6 || rep == 0
        }
    });
    let s = summarize(&m, &c, &r, "complete", vec![], 99).unwrap();
    assert_eq!(s.accuracy_a, Some(0.75));
    assert_eq!(s.accuracy_b, Some(0.75));
    assert_eq!(s.delta, Some(0.0));
    assert_eq!(s.independent_clusters, 12);
    let mut swapped = r.clone();
    for a in &mut swapped {
        a.candidate = 1 - a.candidate;
    }
    let t = summarize(&m, &c, &swapped, "complete", vec![], 99).unwrap();
    assert_eq!(t.delta, s.delta.map(|d| -d));
    assert_eq!(t.interval, s.interval.map(|[l, h]| [-h, -l]));
}
#[test]
fn deterministic_control_sensitivity_crashes_and_missing_pairs() {
    let (m, c) = fixture();
    let r = rows(&m, &c, |_, _, a| a == 1);
    let s = summarize(&m, &c, &r, "complete", vec![], 42).unwrap();
    assert_eq!(s.delta, Some(1.0));
    assert_eq!(s.interval, Some([1.0, 1.0]));
    assert_eq!(s.verdict, "B better on this suite");
    let control = rows(&m, &c, |i, r, _| (i + r) % 2 == 0);
    let s = summarize(&m, &c, &control, "complete", vec![], 42).unwrap();
    assert_eq!(s.delta, Some(0.0));
    assert_eq!(s.interval, Some([0.0, 0.0]));
    let mut crash = rows(&m, &c, |_, _, _| true);
    crash[0].status = Status::Crash;
    crash[0].chosen = None;
    let s = summarize(&m, &c, &crash, "complete", vec![], 42).unwrap();
    assert_eq!(s.candidates[0].passed, 23);
    crash.pop();
    let s = summarize(&m, &c, &crash, "interrupted", vec![], 42).unwrap();
    assert_eq!(s.complete_cases, 11);
    assert!(s.interval.is_none());
    assert_eq!(s.verdict, "inference unavailable");
}
#[test]
fn symmetry_clusters_do_not_inflate_n_and_duplicates_rejected() {
    let (m, mut c) = fixture();
    c[1].cluster = c[0].cluster.clone();
    let mut r = rows(&m, &c, |_, _, _| true);
    let s = summarize(&m, &c, &r, "complete", vec![], 3).unwrap();
    assert_eq!(s.independent_clusters, 11);
    r.push(r[0].clone());
    assert!(summarize(&m, &c, &r, "complete", vec![], 3).is_err());
}
#[test]
fn corruption_before_final_line_is_error() {
    let p = std::env::temp_dir().join(format!("gym-jsonl-{}", timestamp()));
    std::fs::write(&p, b"{bad}\n{truncated").unwrap();
    assert!(read_jsonl::<serde_json::Value>(&p, true).is_err());
    std::fs::remove_file(p).unwrap();
}

#[test]
fn missing_family_never_redistributes_primary_weight() {
    let (m, c) = fixture();
    let r = rows(&m, &c, |_, _, _| true)
        .into_iter()
        .filter(|a| a.case_id.starts_with("f0-"))
        .collect::<Vec<_>>();
    let s = summarize(&m, &c, &r, "interrupted", vec![], 3).unwrap();
    assert!(s.accuracy_a.is_none());
    assert!(s.delta.is_none());
    assert!(s.interval.is_none());
}
