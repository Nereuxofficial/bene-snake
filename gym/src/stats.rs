//! Paired base-cluster scores and seeded within-family clustered bootstrap.
use crate::{artifact::Rng, schema::*};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyScore {
    pub clusters: usize,
    pub cases: usize,
    pub accuracy_a: Option<f64>,
    pub accuracy_b: Option<f64>,
    pub delta: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateMetrics {
    pub attempts: usize,
    pub passed: usize,
    pub statuses: BTreeMap<String, usize>,
    pub latency_p50_ms: Option<f64>,
    pub latency_p95_ms: Option<f64>,
    pub startup_p50_ms: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub schema: u32,
    pub status: String,
    pub planned_attempts: usize,
    pub recorded_attempts: usize,
    pub complete_cases: usize,
    pub independent_clusters: usize,
    pub accuracy_a: Option<f64>,
    pub accuracy_b: Option<f64>,
    pub delta: Option<f64>,
    pub interval: Option<[f64; 2]>,
    pub interval_method: String,
    pub bootstrap_seed: u64,
    pub threshold: f64,
    pub verdict: String,
    pub families: BTreeMap<String, FamilyScore>,
    pub candidates: [CandidateMetrics; 2],
    pub trial_pairs: BTreeMap<String, usize>,
    pub unique_success_accuracy: [Option<f64>; 2],
    pub interruption_gaps: Vec<String>,
    pub measured_move_ms: f64,
    pub measured_startup_ms: f64,
}
pub fn passed(a: &Attempt, c: &Case) -> bool {
    a.status == Status::Valid
        && a.latency_ms <= a.deadline_ms as f64
        && a.chosen.is_some_and(|m| c.labels.get(m) == Label::Success)
}
fn quantile(mut v: Vec<f64>, q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[((v.len() - 1) as f64 * q).round() as usize])
}
pub fn summarize(
    m: &Manifest,
    cases: &[Case],
    attempts: &[Attempt],
    run_status: &str,
    gaps: Vec<String>,
    seed: u64,
) -> Result<Summary> {
    ensure!(m.schema == SCHEMA && m.repeats > 0, "invalid manifest");
    let mut seen = HashSet::new();
    let by_case: HashMap<_, _> = cases.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut rows = HashMap::new();
    for a in attempts {
        ensure!(
            a.schema == SCHEMA
                && a.run_id == m.run_id
                && a.candidate < 2
                && a.repeat < m.repeats
                && by_case.contains_key(a.case_id.as_str())
                && a.binary_hash == m.candidates[a.candidate].binary_hash
                && a.deadline_ms == m.timeout_ms,
            "attempt contract mismatch"
        );
        ensure!(
            seen.insert((&a.case_id, a.repeat, a.candidate)),
            "duplicate attempt key"
        );
        ensure!(
            a.latency_ms.is_finite()
                && a.latency_ms >= 0.0
                && a.startup_ms.is_finite()
                && a.startup_ms >= 0.0,
            "invalid latency"
        );
        rows.insert((a.case_id.as_str(), a.repeat, a.candidate), a);
    }
    let mut family_clusters = BTreeMap::<String, BTreeMap<String, Vec<[f64; 2]>>>::new();
    for family in &m.suite.families {
        family_clusters.insert(family.clone(), BTreeMap::new());
    }
    let mut complete_cases = 0;
    let mut unique = [vec![], vec![]];
    let mut pairs = BTreeMap::from([
        ("both_pass".into(), 0),
        ("both_fail".into(), 0),
        ("a_only".into(), 0),
        ("b_only".into(), 0),
    ]);
    for c in cases {
        let complete =
            (0..m.repeats).all(|r| (0..2).all(|a| rows.contains_key(&(c.id.as_str(), r, a))));
        for r in 0..m.repeats {
            if let (Some(a), Some(b)) = (
                rows.get(&(c.id.as_str(), r, 0)),
                rows.get(&(c.id.as_str(), r, 1)),
            ) {
                let key = match (passed(a, c), passed(b, c)) {
                    (true, true) => "both_pass",
                    (false, false) => "both_fail",
                    (true, false) => "a_only",
                    (false, true) => "b_only",
                };
                *pairs.get_mut(key).unwrap() += 1;
            }
        }
        if !complete {
            continue;
        }
        complete_cases += 1;
        let score = std::array::from_fn(|a| {
            (0..m.repeats)
                .map(|r| passed(rows[&(c.id.as_str(), r, a)], c) as u8 as f64)
                .sum::<f64>()
                / m.repeats as f64
        });
        family_clusters
            .get_mut(&c.family)
            .ok_or_else(|| anyhow::anyhow!("undeclared family"))?
            .entry(c.cluster.clone())
            .or_default()
            .push(score);
        if c.labels.successes().len() == 1 {
            for a in 0..2 {
                unique[a].push(score[a]);
            }
        }
    }
    let mut families = BTreeMap::new();
    let mut points = vec![];
    let mut all_a = vec![];
    let mut all_b = vec![];
    let mut independent = 0;
    for (family, clusters) in family_clusters {
        let data = clusters
            .values()
            .map(|positions| {
                [
                    positions.iter().map(|s| s[0]).sum::<f64>() / positions.len() as f64,
                    positions.iter().map(|s| s[1]).sum::<f64>() / positions.len() as f64,
                ]
            })
            .collect::<Vec<_>>();
        let n = data.len();
        independent += n;
        let a = (n > 0).then(|| data.iter().map(|s| s[0]).sum::<f64>() / n as f64);
        let b = (n > 0).then(|| data.iter().map(|s| s[1]).sum::<f64>() / n as f64);
        if let (Some(a), Some(b)) = (a, b) {
            all_a.push(a);
            all_b.push(b);
        }
        families.insert(
            family,
            FamilyScore {
                clusters: n,
                cases: clusters.values().map(Vec::len).sum(),
                accuracy_a: a,
                accuracy_b: b,
                delta: a.zip(b).map(|(a, b)| b - a),
            },
        );
        points.push(data);
    }
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let accuracy_a = (all_a.len() == points.len())
        .then(|| mean(&all_a))
        .flatten();
    let accuracy_b = (all_b.len() == points.len())
        .then(|| mean(&all_b))
        .flatten();
    let delta = accuracy_a.zip(accuracy_b).map(|(a, b)| b - a);
    let complete = complete_cases == cases.len()
        && attempts.len() == cases.len() * m.repeats * 2
        && run_status == "complete";
    let inference = complete
        && points
            .iter()
            .all(|f| f.len() >= m.suite.min_clusters_per_family)
        && m.suite.bootstrap_samples >= 1000;
    let interval = if inference {
        let mut rng = Rng(seed);
        let mut bootstrap = vec![];
        for _ in 0..m.suite.bootstrap_samples {
            let mut total = 0.0;
            for f in &points {
                let mut sum = 0.0;
                for _ in 0..f.len() {
                    let s = f[rng.range(f.len())];
                    sum += s[1] - s[0];
                }
                total += sum / f.len() as f64;
            }
            bootstrap.push(total / points.len() as f64);
        }
        bootstrap.sort_by(f64::total_cmp);
        Some([
            bootstrap[((bootstrap.len() - 1) as f64 * 0.025).round() as usize],
            bootstrap[((bootstrap.len() - 1) as f64 * 0.975).round() as usize],
        ])
    } else {
        None
    };
    let verdict = match interval {
        Some([lo, _]) if lo > m.suite.threshold => "B better on this suite",
        Some([_, hi]) if hi < -m.suite.threshold => "A better on this suite",
        Some(_) => "inconclusive",
        None => "inference unavailable",
    }
    .into();
    let candidates = std::array::from_fn(|candidate| {
        let v = attempts
            .iter()
            .filter(|a| a.candidate == candidate)
            .collect::<Vec<_>>();
        let mut statuses = BTreeMap::new();
        for a in &v {
            *statuses
                .entry(
                    serde_json::to_value(a.status)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                )
                .or_default() += 1;
        }
        let latencies = v
            .iter()
            .filter(|a| a.status != Status::StartupFailure)
            .map(|a| a.latency_ms)
            .collect::<Vec<_>>();
        CandidateMetrics {
            attempts: v.len(),
            passed: v
                .iter()
                .filter(|a| passed(a, by_case[a.case_id.as_str()]))
                .count(),
            statuses,
            latency_p50_ms: quantile(latencies.clone(), 0.5),
            latency_p95_ms: quantile(latencies, 0.95),
            startup_p50_ms: quantile(v.iter().map(|a| a.startup_ms).collect(), 0.5),
        }
    });
    Ok(Summary {
        schema: SCHEMA,
        status: if run_status == "invalid" {
            "invalid"
        } else if complete {
            "complete"
        } else {
            "interrupted"
        }
        .into(),
        planned_attempts: cases.len() * m.repeats * 2,
        recorded_attempts: attempts.len(),
        complete_cases,
        independent_clusters: independent,
        accuracy_a,
        accuracy_b,
        delta,
        interval,
        interval_method: format!(
            "95% paired base-cluster bootstrap within families; {} replicates; equal family weights; minimum {} clusters/family",
            m.suite.bootstrap_samples, m.suite.min_clusters_per_family
        ),
        bootstrap_seed: seed,
        threshold: m.suite.threshold,
        verdict,
        families,
        candidates,
        trial_pairs: pairs,
        unique_success_accuracy: [mean(&unique[0]), mean(&unique[1])],
        interruption_gaps: gaps,
        measured_move_ms: attempts.iter().map(|a| a.latency_ms).sum(),
        measured_startup_ms: attempts.iter().map(|a| a.startup_ms).sum(),
    })
}
