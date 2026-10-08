use gym::{artifact::*, candidate, generate, runner, schema::*, trials};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gym-{name}-{}-{}", std::process::id(), timestamp()))
}
fn mock(out: &Path, mode: &str, mv: Move) -> PathBuf {
    let p = out.join(format!("mock-{mode}"));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_server.py");
    std::fs::write(&p,format!("#!/usr/bin/python3\nimport runpy,sys\nsys.argv=[{:?},{:?},{:?}]\nrunpy.run_path({:?},run_name='__main__')\n",fixture.display().to_string(),mode,serde_json::to_value(mv).unwrap().as_str().unwrap(),fixture.display().to_string())).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    p
}
#[tokio::test]
async fn exact_statuses_lifecycle_descendants_and_no_overlap() -> anyhow::Result<()> {
    let out = temp("trials");
    let s = generate::suite("balanced-v1")?;
    assert!(generate::generate(&out, s, 6, 1234)?.complete);
    let (_, cases) = generate::load(&out)?;
    let case = &cases[0];
    let good = case.labels.successes()[0];
    let modes = [
        ("valid", Status::Valid),
        ("late", Status::Timeout),
        ("partial", Status::Timeout),
        ("malformed", Status::Malformed),
        ("oversized", Status::Malformed),
        ("http_error", Status::HttpError),
        ("crash", Status::Crash),
        ("hung_end", Status::Valid),
        ("descendant", Status::Valid),
        ("hung_startup", Status::StartupFailure),
    ];
    for (index, (mode, status)) in modes.iter().enumerate() {
        let bin = mock(&out, mode, good);
        let run = out.join(mode);
        std::fs::create_dir_all(run.join("runtime"))?;
        std::fs::create_dir_all(run.join("logs"))?;
        let c = candidate::prepare(Path::new("."), &run, &format!("bin:{}", bin.display()), 0)?;
        let m = Manifest {
            schema: SCHEMA,
            run_id: "test".into(),
            created: timestamp(),
            corpus_hash: String::new(),
            generation_hash: String::new(),
            harness_hash: String::new(),
            rules_hash: rules_hash(),
            oracle_hash: oracle_hash(),
            candidates: [c.clone(), c],
            repeats: 1,
            timeout_ms: 60,
            suite: generate::suite("balanced-v1")?,
            machine: String::new(),
            mode: "cold".into(),
            environment: Default::default(),
            build_ms: 0,
        };
        if *mode != "hung_startup" {
            trials::preflight(&run, &m.candidates[0], case).await?;
        }
        let before = std::time::Instant::now();
        let a = trials::trial(&run, &m, case, 0, 0, index).await?;
        assert_eq!(a.status, *status, "{mode}: {}", a.detail);
        assert!(before.elapsed().as_secs() < 7, "cleanup took too long");
        if a.status == Status::Timeout {
            assert!(a.chosen.is_none());
        }
        if *mode == "descendant" {
            let pid = std::fs::read_to_string(run.join("runtime/descendant.pid"))?;
            let proc = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
            assert!(
                std::fs::read_to_string(proc)
                    .map_or(true, |s| s.split_whitespace().nth(2) == Some("Z"))
            );
        }
        candidate::check(&run, &m.candidates[0])?;
    }
    std::fs::remove_dir_all(out)?;
    Ok(())
}
#[tokio::test]
async fn resumable_run_hashes_and_final_truncation() -> anyhow::Result<()> {
    let out = temp("resume");
    assert!(generate::generate(&out, generate::suite("balanced-v1")?, 6, 1234)?.complete);
    let binary = mock(&out, "valid", Move::Right);
    let descriptor = format!("bin:{}", binary.display());
    runner::initialize(Path::new("."), &out, &descriptor, &descriptor, 1, 100)?;
    let (m, cases, _) = runner::audit(&out, true)?;
    trials::preflight(&out, &m.candidates[0], &cases[0]).await?;
    let first = trials::trial(&out, &m, &cases[0], 0, 0, 0).await?;
    append(&out.join("attempts.jsonl"), &first)?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(out.join("attempts.jsonl"))?
        .write_all(b"{truncated")?;
    let s = runner::execute(&out, true).await?;
    assert_eq!(s.status, "complete");
    assert_eq!(s.recorded_attempts, 12);
    assert_eq!(s.complete_cases, 6);
    assert_eq!(s.interruption_gaps.len(), 1);
    assert_eq!(runner::audit(&out, true)?.2.len(), 12);
    let pid_before = std::fs::read_to_string(out.join("runtime/parent.pid"))?;
    assert_eq!(runner::execute(&out, true).await?.recorded_attempts, 12);
    assert_eq!(
        std::fs::read_to_string(out.join("runtime/parent.pid"))?,
        pid_before,
        "resuming a completed run must not start new candidates"
    );
    let frozen = out.join(&m.candidates[0].binary);
    std::fs::set_permissions(&frozen, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(&frozen, b"changed")?;
    assert!(runner::audit(&out, true).is_err());
    std::fs::remove_dir_all(out)?;
    Ok(())
}
#[test]
fn ab_ba_schedule_is_predeclared() {
    let out = temp("schedule");
    generate::generate(&out, generate::suite("balanced-v1").unwrap(), 12, 1234).unwrap();
    let (_, cases) = generate::load(&out).unwrap();
    let s = trials::schedule(&cases, 3);
    assert_eq!(
        &s[..6],
        &[
            (0, 0, 0),
            (0, 0, 1),
            (0, 1, 1),
            (0, 1, 0),
            (0, 2, 0),
            (0, 2, 1)
        ]
    );
    assert_eq!(&s[6..8], &[(1, 0, 1), (1, 0, 0)]);
    std::fs::remove_dir_all(out).unwrap();
}

#[tokio::test]
async fn cancellation_drops_process_group_during_startup() -> anyhow::Result<()> {
    let out = temp("cancel");
    assert!(generate::generate(&out, generate::suite("balanced-v1")?, 6, 1234)?.complete);
    let binary = mock(&out, "hung_startup", Move::Right);
    let d = format!("bin:{}", binary.display());
    runner::initialize(Path::new("."), &out, &d, &d, 1, 100)?;
    std::fs::create_dir_all(out.join("runtime"))?;
    let (m, cases, _) = runner::audit(&out, true)?;
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        trials::trial(&out, &m, &cases[0], 0, 0, 0),
    )
    .await;
    assert!(result.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let pid = std::fs::read_to_string(out.join("runtime/parent.pid"))?;
    let proc = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
    assert!(
        std::fs::read_to_string(proc).map_or(true, |s| s.split_whitespace().nth(2) == Some("Z"))
    );
    std::fs::remove_dir_all(out)?;
    Ok(())
}
