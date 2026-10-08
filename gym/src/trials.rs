//! Cold sequential production HTTP trials, bounded client round-trip deadlines,
//! and process-group teardown. No direct candidate library imports.
use crate::{artifact::*, rules, schema::*};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    task::JoinHandle,
};
const RESPONSE_LIMIT: usize = 65536;
const LOG_LIMIT: usize = 1024 * 1024;
const STARTUP: Duration = Duration::from_secs(5);
const CLEANUP: Duration = Duration::from_millis(250);

pub struct Process {
    pub child: Child,
    pub port: u16,
    pid: i32,
    logs: Vec<JoinHandle<()>>,
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.pid > 0 {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}
async fn drain(reader: impl AsyncRead + Unpin, path: std::path::PathBuf) {
    let Ok(mut f) = tokio::fs::File::create(path).await else {
        return;
    };
    let mut reader = reader;
    let mut b = [0; 8192];
    let mut kept = 0;
    while let Ok(n) = reader.read(&mut b).await {
        if n == 0 {
            break;
        }
        let count = n.min(LOG_LIMIT - kept);
        if count > 0 {
            let _ = f.write_all(&b[..count]).await;
            kept += count;
        }
    }
}
impl Process {
    pub async fn launch(binary: &Path, cwd: &Path, log: &Path) -> Result<Self> {
        // dotenvy searches ancestors. An empty local file stops it from loading
        // the checkout's .env even though the child's inherited env is cleared.
        tokio::fs::write(cwd.join(".env"), b"").await?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let mut cmd = Command::new(binary);
        cmd.current_dir(cwd)
            .env_clear()
            .env("PORT", port.to_string())
            .env("GLITCHTIP_KEY", "")
            .env("RUST_LOG", "warn")
            .env("TZ", "UTC")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd.process_group(0);
        let mut child = cmd.spawn().context("launching candidate")?;
        let pid = child.id().unwrap() as i32;
        let logs = vec![
            tokio::spawn(drain(
                child.stdout.take().unwrap(),
                log.with_extension("stdout.log"),
            )),
            tokio::spawn(drain(
                child.stderr.take().unwrap(),
                log.with_extension("stderr.log"),
            )),
        ];
        Ok(Self {
            child,
            port,
            pid,
            logs,
        })
    }
    pub async fn stop(&mut self) {
        if self.pid > 0 {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
            self.pid = 0;
        }
        let _ = self.child.wait().await;
        for t in self.logs.drain(..) {
            let _ = t.await;
        }
    }
    pub async fn ready(&mut self, client: &reqwest::Client) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < STARTUP {
            ensure!(
                self.child.try_wait()?.is_none(),
                "candidate exited during startup"
            );
            for path in ["/info", "/"] {
                if http(client, self.port, path, None, Duration::from_millis(100))
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        anyhow::bail!("startup readiness deadline exceeded")
    }
}
#[derive(Debug)]
pub enum HttpFailure {
    Timeout,
    Http(String),
    Oversized,
    Io(String),
}
impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HttpFailure {}
pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
pub async fn http(
    client: &reqwest::Client,
    port: u16,
    path: &str,
    body: Option<&Request>,
    budget: Duration,
) -> std::result::Result<Vec<u8>, HttpFailure> {
    let start = Instant::now();
    let work = async {
        let url = format!("http://127.0.0.1:{port}{path}");
        let req = if let Some(b) = body {
            client.post(url).json(b)
        } else {
            client.get(url)
        };
        let mut response = req
            .send()
            .await
            .map_err(|e| HttpFailure::Io(e.to_string()))?;
        if !response.status().is_success() {
            return Err(HttpFailure::Http(response.status().to_string()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > RESPONSE_LIMIT as u64)
        {
            return Err(HttpFailure::Oversized);
        }
        let mut data = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| HttpFailure::Io(e.to_string()))?
        {
            if data.len() + chunk.len() > RESPONSE_LIMIT {
                return Err(HttpFailure::Oversized);
            }
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    };
    let result = tokio::time::timeout(budget, work)
        .await
        .map_err(|_| HttpFailure::Timeout)?;
    if start.elapsed() > budget {
        return Err(HttpFailure::Timeout);
    }
    result
}
pub async fn preflight(run: &Path, c: &Candidate, case: &Case) -> Result<()> {
    let cwd = run.join("runtime");
    tokio::fs::create_dir_all(&cwd).await?;
    let client = client()?;
    let mut p = Process::launch(&run.join(&c.binary), &cwd, &run.join("logs/preflight")).await?;
    let result = async {
        p.ready(&client).await?;
        http(
            &client,
            p.port,
            "/start",
            Some(&case.request),
            Duration::from_secs(2),
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let _ = http(&client, p.port, "/end", Some(&case.request), CLEANUP).await;
    p.stop().await;
    result.with_context(|| format!("preflight {} failed; comparison invalid", c.descriptor))
}
pub fn schedule(cases: &[Case], repeats: usize) -> Vec<(usize, usize, usize)> {
    let mut schedule = vec![];
    let mut families = BTreeMap::<&str, usize>::new();
    for (i, c) in cases.iter().enumerate() {
        let ordinal = families.entry(&c.family).or_default();
        for rep in 0..repeats {
            let order = if (*ordinal + rep).is_multiple_of(2) {
                [0, 1]
            } else {
                [1, 0]
            };
            for candidate in order {
                schedule.push((i, rep, candidate));
            }
        }
        *ordinal += 1;
    }
    schedule
}
pub async fn trial(
    run: &Path,
    m: &Manifest,
    case: &Case,
    rep: usize,
    candidate: usize,
    order: usize,
) -> Result<Attempt> {
    let c = &m.candidates[candidate];
    let mut request = case.request.clone();
    request.game.timeout = m.timeout_ms;
    request.game.id = format!("{}-{}-{rep}-{candidate}", m.run_id, case.id);
    let log = format!("logs/attempt-{order}");
    let mut a = Attempt {
        schema: SCHEMA,
        run_id: m.run_id.clone(),
        case_id: case.id.clone(),
        candidate,
        repeat: rep,
        order,
        request_hash: value_hash(&request)?,
        binary_hash: c.binary_hash.clone(),
        status: Status::StartupFailure,
        chosen: None,
        response: None,
        latency_ms: 0.0,
        startup_ms: 0.0,
        deadline_ms: m.timeout_ms,
        timestamp: timestamp(),
        log: log.clone(),
        detail: String::new(),
    };
    let client = client()?;
    let start = Instant::now();
    let mut p =
        Process::launch(&run.join(&c.binary), &run.join("runtime"), &run.join(&log)).await?;
    let startup = async {
        p.ready(&client).await?;
        http(
            &client,
            p.port,
            "/start",
            Some(&request),
            Duration::from_secs(2),
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    a.startup_ms = start.elapsed().as_secs_f64() * 1000.0;
    if let Err(e) = startup {
        a.detail = e.to_string();
        p.stop().await;
        return Ok(a);
    }
    let start = Instant::now();
    let response = http(
        &client,
        p.port,
        "/move",
        Some(&request),
        Duration::from_millis(m.timeout_ms),
    )
    .await;
    a.latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    match response {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(v) => {
                let chosen = v
                    .get("move")
                    .and_then(|mv| serde_json::from_value::<Move>(mv.clone()).ok());
                a.chosen = chosen;
                a.response = Some(v);
                a.status = if let Some(mv) = chosen {
                    let s = rules::State::from_request(&request);
                    if rules::responses(&s, &request.you.id, mv).iter().all(|b| {
                        !rules::advance(&s, b, &request.you.id, &case.objective)
                            .state
                            .snakes
                            .iter()
                            .any(|s| s.id == request.you.id)
                    }) {
                        Status::Fatal
                    } else {
                        Status::Valid
                    }
                } else {
                    Status::Malformed
                };
            }
            Err(e) => {
                a.status = Status::Malformed;
                a.detail = e.to_string();
            }
        },
        Err(e) => {
            a.status = match &e {
                HttpFailure::Timeout => Status::Timeout,
                HttpFailure::Oversized => Status::Malformed,
                HttpFailure::Http(_) => Status::HttpError,
                HttpFailure::Io(_) => {
                    if p.child.try_wait()?.is_some() {
                        Status::Crash
                    } else {
                        Status::HttpError
                    }
                }
            };
            a.detail = e.to_string();
        }
    }
    let _ = http(&client, p.port, "/end", Some(&request), CLEANUP).await;
    p.stop().await;
    Ok(a)
}
