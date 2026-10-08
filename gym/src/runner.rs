//! Durable orchestration: freeze corpus/identities before consulting candidates.
use crate::{
    artifact::*,
    candidate, generate,
    schema::*,
    stats::{self, Summary},
    trials,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    time::Instant,
};
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunState {
    pub schema: u32,
    pub status: String,
    pub gaps: Vec<String>,
    pub detail: String,
}
fn state(out: &Path, status: &str, gaps: Vec<String>, detail: String) -> Result<()> {
    write_json(
        &out.join("state.json"),
        &RunState {
            schema: SCHEMA,
            status: status.into(),
            gaps,
            detail,
        },
    )
}
pub fn freeze_corpus(from: &Path, out: &Path) -> Result<()> {
    let (g, cases) = generate::load(from)?;
    ensure!(g.complete, "corpus quotas incomplete");
    fs::create_dir_all(out.join("certificates"))?;
    for name in ["generation.json", "corpus.jsonl"] {
        fs::copy(from.join(name), out.join(name))?;
    }
    for c in cases {
        let file = format!("{}.json", c.certificate_hash);
        fs::copy(
            from.join("certificates").join(&file),
            out.join("certificates").join(&file),
        )?;
    }
    Ok(())
}
pub fn initialize(
    repo: &Path,
    out: &Path,
    a: &str,
    b: &str,
    repeats: usize,
    timeout_ms: u64,
) -> Result<()> {
    ensure!(
        repeats > 0 && timeout_ms > 0,
        "repeats/timeout must be positive"
    );
    ensure!(
        !out.join("manifest.json").exists(),
        "run exists; use resume"
    );
    let (g, _) = generate::load(out)?;
    ensure!(g.complete, "corpus incomplete; inspect generation.json");
    let start = Instant::now();
    let harness = std::env::current_exe()?;
    fs::create_dir_all(out.join("binaries"))?;
    let frozen_harness = out.join("binaries/snake-gym");
    fs::copy(&harness, &frozen_harness)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&frozen_harness, fs::Permissions::from_mode(0o500))?;
    let ca = candidate::prepare(repo, out, a, 0)?;
    let cb = candidate::prepare(repo, out, b, 1)?;
    let m=Manifest{schema:SCHEMA,run_id:format!("{}-{}",timestamp(),&g.corpus_hash[..12]),created:timestamp(),corpus_hash:g.corpus_hash,generation_hash:file_hash(&out.join("generation.json"))?,harness_hash:file_hash(&std::env::current_exe()?)?,rules_hash:rules_hash(),oracle_hash:oracle_hash(),candidates:[ca,cb],repeats,timeout_ms,suite:g.suite,machine:candidate::machine()?,mode:"cold; sequential; synthetic standard 11x11; no future food; adversarial coalition; server may bind 0.0.0.0".into(),environment:BTreeMap::from([("PORT".into(),"unique ephemeral port per process".into()),("GLITCHTIP_KEY".into(),"empty (disabled telemetry)".into()),("RUST_LOG".into(),"warn".into()),("TZ".into(),"UTC".into()),("inherited environment".into(),"cleared; empty runtime .env stops ancestor dotenv loading".into()),("worker count".into(),"implemented by each binary; unobserved".into())]),build_ms:start.elapsed().as_millis() as u64};
    write_json(&out.join("manifest.json"), &m)?;
    atomic(
        &out.join("manifest.sha256"),
        file_hash(&out.join("manifest.json"))?.as_bytes(),
    )?;
    state(out, "interrupted", vec![], "not started".into())?;
    Ok(())
}
pub fn audit(out: &Path, check_harness: bool) -> Result<(Manifest, Vec<Case>, Vec<Attempt>)> {
    let m: Manifest = read_json(&out.join("manifest.json"))?;
    ensure!(
        m.schema == SCHEMA && m.repeats > 0 && m.timeout_ms > 0,
        "incompatible manifest"
    );
    ensure!(
        fs::read_to_string(out.join("manifest.sha256"))? == file_hash(&out.join("manifest.json"))?,
        "manifest/settings hash mismatch"
    );
    ensure!(
        m.generation_hash == file_hash(&out.join("generation.json"))?,
        "generation settings mismatch"
    );
    let (g, cases) = generate::load(out)?;
    ensure!(
        g.complete
            && g.corpus_hash == m.corpus_hash
            && m.rules_hash == rules_hash()
            && m.oracle_hash == oracle_hash(),
        "corpus/oracle mismatch"
    );
    if check_harness {
        ensure!(
            m.harness_hash == file_hash(&std::env::current_exe()?)?,
            "harness binary changed; use frozen harness to resume"
        );
        for c in &m.candidates {
            candidate::check(out, c)?;
        }
    }
    let attempts: Vec<Attempt> = read_jsonl(&out.join("attempts.jsonl"), check_harness)?;
    let schedule = trials::schedule(&cases, m.repeats);
    let mut order_keys = HashSet::new();
    for a in &attempts {
        let (case, rep, candidate) = *schedule.get(a.order).context("unexpected order index")?;
        let c = &cases[case];
        ensure!(
            a.case_id == c.id
                && a.repeat == rep
                && a.candidate == candidate
                && order_keys.insert(a.order),
            "attempt schedule mismatch/duplicate"
        );
        let mut r = c.request.clone();
        r.game.timeout = m.timeout_ms;
        r.game.id = format!("{}-{}-{rep}-{candidate}", m.run_id, c.id);
        ensure!(a.request_hash == value_hash(&r)?, "request hash mismatch");
    }
    stats::summarize(&m, &cases, &attempts, "interrupted", vec![], g.seed)?;
    Ok((m, cases, attempts))
}
pub fn summary(out: &Path) -> Result<Summary> {
    let (m, cases, attempts) = audit(out, false)?;
    let st: RunState = read_json(&out.join("state.json"))?;
    ensure!(st.schema == SCHEMA, "incompatible run state");
    let g: Generation = read_json(&out.join("generation.json"))?;
    let s = stats::summarize(
        &m,
        &cases,
        &attempts,
        &st.status,
        st.gaps,
        g.seed ^ 0xb00757a9,
    )?;
    write_json(&out.join("summary.json"), &s)?;
    Ok(s)
}
pub async fn execute(out: &Path, resume: bool) -> Result<Summary> {
    let _lock = RunLock::acquire(out)?;
    let (m, cases, attempts) = audit(out, true)?;
    let old: RunState = read_json(&out.join("state.json"))?;
    ensure!(
        old.status != "invalid",
        "invalid comparison: {}",
        old.detail
    );
    if old.status == "complete" && attempts.len() == cases.len() * m.repeats * 2 {
        return summary(out);
    }
    let mut gaps = old.gaps;
    if resume && attempts.len() < cases.len() * m.repeats * 2 {
        gaps.push(format!(
            "resume at {}; {} previously recorded attempts; original schedule retained",
            timestamp(),
            attempts.len()
        ));
    }
    state(
        out,
        "interrupted",
        gaps.clone(),
        "active run; incomplete until all attempts durable".into(),
    )?;
    tokio::select! {result=measure(out,m,cases,attempts,gaps.clone())=>result,_=tokio::signal::ctrl_c()=>{state(out,"interrupted",gaps,"cancelled; incomplete blocks excluded".into())?;summary(out)}}
}
async fn measure(
    out: &Path,
    m: Manifest,
    cases: Vec<Case>,
    attempts: Vec<Attempt>,
    gaps: Vec<String>,
) -> Result<Summary> {
    let client_start = Instant::now();
    for c in &m.candidates {
        if let Err(e) = trials::preflight(out, c, &cases[0]).await {
            state(out, "invalid", gaps, e.to_string())?;
            return Err(e);
        }
    }
    state(
        out,
        "interrupted",
        gaps.clone(),
        "active run; incomplete until all scheduled attempts are durable".into(),
    )?;
    let done: HashSet<_> = attempts.iter().map(|a| a.order).collect();
    let schedule = trials::schedule(&cases, m.repeats);
    let mut recorded = done.len();
    for (order, (case, rep, candidate)) in schedule.iter().copied().enumerate() {
        if done.contains(&order) {
            continue;
        }
        let result = trials::trial(out, &m, &cases[case], rep, candidate, order).await;
        match result {
            Ok(a) => append(&out.join("attempts.jsonl"), &a)?,
            Err(e) => {
                state(out, "interrupted", gaps, e.to_string())?;
                return Err(e);
            }
        }
        recorded += 1;
        if recorded.is_multiple_of(12) || recorded == schedule.len() {
            let elapsed = client_start.elapsed().as_secs_f64();
            eprintln!(
                "run: {recorded}/{} attempts; {:.1}s elapsed; {:.1}s projected remaining",
                schedule.len(),
                elapsed,
                elapsed / (recorded - done.len()) as f64 * (schedule.len() - recorded) as f64
            );
        }
    }
    state(out, "complete", gaps, "all planned attempts durable".into())?;
    summary(out)
}

struct RunLock(std::fs::File);
impl RunLock {
    fn acquire(out: &Path) -> Result<Self> {
        use std::os::fd::AsRawFd;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(out.join("run.lock"))?;
        ensure!(
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "another process owns this run"
        );
        Ok(Self(f))
    }
}
impl Drop for RunLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
