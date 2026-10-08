use gym::{artifact::*, generate, report, runner, schema::*, trials};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
#[test]
fn offline_report_totals_escape_and_no_move_fixture() -> anyhow::Result<()> {
    let out =
        std::env::temp_dir().join(format!("gym-report-{}-{}", std::process::id(), timestamp()));
    assert!(generate::generate(&out, generate::suite("balanced-v1")?, 6, 1234)?.complete);
    let bin = out.join("fixture");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n")?;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700))?;
    let d = format!("bin:{}", bin.display());
    runner::initialize(std::path::Path::new("."), &out, &d, &d, 1, 500)?;
    let (mut m, cases, _) = runner::audit(&out, true)?;
    m.candidates[0].descriptor = "</script><img src=x onerror=alert(1)>".into();
    m.mode = "test-only synthetic attempt fixture: success, regression, both fail, timeout".into();
    write_json(&out.join("manifest.json"), &m)?;
    atomic(
        &out.join("manifest.sha256"),
        file_hash(&out.join("manifest.json"))?.as_bytes(),
    )?;
    for (order, (i, rep, candidate)) in trials::schedule(&cases, 1).into_iter().enumerate() {
        let c = &cases[i];
        let mut req = c.request.clone();
        req.game.timeout = 500;
        req.game.id = format!("{}-{}-{rep}-{candidate}", m.run_id, c.id);
        let success = (i % 4 == 0) || (i % 4 == 1 && candidate == 0);
        let timeout = i % 4 == 3 && candidate == 1;
        let chosen = if timeout {
            None
        } else if success {
            Some(c.labels.successes()[0])
        } else {
            Move::ALL
                .into_iter()
                .find(|mv| c.labels.get(*mv) == Label::Failure)
        };
        append(
            &out.join("attempts.jsonl"),
            &Attempt {
                schema: SCHEMA,
                run_id: m.run_id.clone(),
                case_id: c.id.clone(),
                candidate,
                repeat: rep,
                order,
                request_hash: value_hash(&req)?,
                binary_hash: m.candidates[candidate].binary_hash.clone(),
                status: if timeout {
                    Status::Timeout
                } else {
                    Status::Valid
                },
                chosen,
                response: None,
                latency_ms: if timeout { 501.0 } else { 25.0 },
                startup_ms: 10.0,
                deadline_ms: 500,
                timestamp: timestamp(),
                log: String::new(),
                detail: "synthetic report fixture".into(),
            },
        )?;
    }
    write_json(
        &out.join("state.json"),
        &runner::RunState {
            schema: SCHEMA,
            status: "complete".into(),
            gaps: vec![],
            detail: "synthetic report fixture".into(),
        },
    )?;
    let path = report::export(&out)?;
    let html = std::fs::read_to_string(path)?;
    assert!(!html.contains("</script><img src=x"));
    assert!(!html.contains("fetch("));
    assert!(!html.contains("https://"));
    let data = html
        .split("<script type=\"application/json\" id=\"data\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(data)?;
    assert_eq!(
        data["manifest"]["candidates"][0]["descriptor"],
        m.candidates[0].descriptor
    );
    assert_eq!(
        data["summary"],
        serde_json::to_value(runner::summary(&out)?)?
    );
    assert_eq!(data["attempts"].as_array().unwrap().len(), 12);
    assert_eq!(data["cases"].as_array().unwrap().len(), 6);
    assert!(
        data["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["status"] == "timeout" && a["chosen"].is_null())
    );
    std::fs::remove_dir_all(out)?;
    Ok(())
}
#[test]
fn incompatible_schema_rejected() {
    let file: PathBuf = std::env::temp_dir().join(format!("gym-schema-{}", timestamp()));
    std::fs::write(&file, "{\"schema\":99}").unwrap();
    assert!(read_json::<Manifest>(&file).is_err());
    std::fs::remove_file(file).unwrap();
}
