//! Offline report export and read-only loopback serving. No process launch routes.
use crate::{
    artifact::*,
    oracle::{self, Certificate},
    runner,
    schema::*,
    stats,
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{net::SocketAddr, path::Path};
pub fn export(out: &Path) -> Result<std::path::PathBuf> {
    let summary = runner::summary(out)?;
    let (m, cases, attempts) = runner::audit(out, false)?;
    let generation: Generation = read_json(&out.join("generation.json"))?;
    let mut views = vec![];
    for c in &cases {
        let cert: Certificate = read_json(
            &out.join("certificates")
                .join(format!("{}.json", c.certificate_hash)),
        )?;
        let continuations = Move::ALL
            .into_iter()
            .map(|mv| (mv, oracle::example(&cert, mv)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let scores: [Option<f64>; 2] = std::array::from_fn(|candidate| {
            let rows = attempts
                .iter()
                .filter(|a| a.case_id == c.id && a.candidate == candidate)
                .collect::<Vec<_>>();
            (rows.len() == m.repeats).then(|| {
                rows.iter().filter(|a| stats::passed(a, c)).count() as f64 / m.repeats as f64
            })
        });
        views.push(json!({"case":c,"continuations":continuations,"scores":scores}));
    }
    let data = json!({"manifest":m,"summary":summary,"generation":generation,"cases":views,"attempts":attempts});
    let embedded = serde_json::to_string(&data)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    ensure!(
        embedded.len() < 128 * 1024 * 1024,
        "report data exceeds 128 MiB; use smaller corpus"
    );
    let html = include_str!("report.html").replace("__DATA__", &embedded);
    let path = out.join("report/index.html");
    atomic(&path, html.as_bytes())?;
    Ok(path)
}
pub async fn serve(out: &Path, bind: SocketAddr) -> Result<()> {
    ensure!(
        bind.ip().is_loopback(),
        "report server requires loopback bind"
    );
    export(out)?;
    let root = out.canonicalize()?;
    let app = axum::Router::new()
        .route(
            "/",
            axum::routing::get({
                let root = root.clone();
                move || {
                    let root = root.clone();
                    async move {
                        match tokio::fs::read(root.join("report/index.html")).await {
                            Ok(bytes) => ([("content-type", "text/html; charset=utf-8")], bytes)
                                .into_response(),
                            Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                }
            }),
        )
        .route(
            "/certificates/{hash}",
            axum::routing::get(
                move |axum::extract::Path(hash): axum::extract::Path<String>| {
                    let root = root.clone();
                    async move {
                        if hash.len() != 69
                            || !hash.ends_with(".json")
                            || !hash.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
                        {
                            return axum::http::StatusCode::NOT_FOUND.into_response();
                        }
                        match tokio::fs::read(root.join("certificates").join(hash)).await {
                            Ok(bytes) => {
                                ([("content-type", "application/json")], bytes).into_response()
                            }
                            Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("Report: http://{}/", listener.local_addr()?);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
use axum::response::IntoResponse;
