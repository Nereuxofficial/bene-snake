//! Trace the origin request lifecycle; consuming a body is not client acknowledgement.
use axum::{
    body::{Body, Bytes, HttpBody},
    extract::Request,
    middleware::Next,
    response::Response,
};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
tokio::task_local! { static CURRENT: MoveTrace; }

#[derive(Clone)]
pub struct MoveTrace(Arc<Trace>);

struct Trace {
    id: u64,
    started: Instant,
    details: Mutex<Details>,
}

struct Details {
    game_id: Option<String>,
    turn: Option<i32>,
    phase: &'static str,
    phase_started: Instant,
    timings: BTreeMap<&'static str, Duration>,
    outcome: &'static str,
}

impl MoveTrace {
    pub fn id(&self) -> u64 {
        self.0.id
    }
    pub fn current() -> Self {
        CURRENT
            .try_with(Clone::clone)
            .unwrap_or_else(|_| Self::new())
    }

    fn new() -> Self {
        let started = Instant::now();
        Self(Arc::new(Trace {
            id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            started,
            details: Mutex::new(Details {
                game_id: None,
                turn: None,
                phase: "reading_body",
                phase_started: started,
                timings: BTreeMap::new(),
                outcome: "active",
            }),
        }))
    }

    pub fn identify(&self, game_id: &str, turn: i32) {
        let mut details = self.0.details.lock().unwrap();
        details.game_id = Some(game_id.to_owned());
        details.turn = Some(turn);
    }

    pub fn phase(&self, phase: &'static str) {
        let mut details = self.0.details.lock().unwrap();
        let previous = details.phase;
        let elapsed = details.phase_started.elapsed();
        *details.timings.entry(previous).or_default() += elapsed;
        details.phase = phase;
        details.phase_started = Instant::now();
    }

    pub fn event(&self, event: &'static str) {
        let details = self.0.details.lock().unwrap();
        tracing::info!(request_id = self.0.id, game_id = details.game_id.as_deref().unwrap_or("unparsed"), turn = details.turn.unwrap_or(-1),
            phase = details.phase, elapsed = ?self.0.started.elapsed(),
            phase_elapsed = ?details.phase_started.elapsed(), timings = ?details.timings,
            event, "Move request lifecycle");
    }

    pub fn guard(&self) -> HandlerGuard {
        HandlerGuard(self.clone())
    }

    pub fn worker_guard(&self) -> WorkerGuard {
        self.event("worker_started");
        WorkerGuard(self.clone())
    }

    fn finish(&self, outcome: &'static str) {
        let mut details = self.0.details.lock().unwrap();
        if details.outcome != "active" {
            return;
        }
        details.outcome = outcome;
        drop(details);
        self.event(outcome);
    }

    fn response(&self, response: Response) -> Response {
        let (mut parts, body) = response.into_parts();
        parts
            .headers
            .insert("x-bene-request-id", self.0.id.to_string().parse().unwrap());
        let status = parts.status.as_u16();
        // A transport need not poll an already-empty body at all.
        let complete = body.is_end_stream();
        Response::from_parts(
            parts,
            Body::new(ObservedBody {
                body,
                trace: self.clone(),
                status,
                complete,
                failed: false,
                bytes: 0,
            }),
        )
    }
}

pub struct HandlerGuard(MoveTrace);
pub struct WorkerGuard(MoveTrace);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.event(if std::thread::panicking() {
            "worker_panicked"
        } else {
            "worker_returned"
        });
    }
}
impl HandlerGuard {
    pub fn finish(self) {
        self.0.finish("handler_returned");
    }
}
impl Drop for HandlerGuard {
    fn drop(&mut self) {
        self.0.finish(if std::thread::panicking() {
            "handler_panicked"
        } else {
            "handler_cancelled"
        });
    }
}

pub async fn observe(request: Request, next: Next) -> Response {
    let trace = MoveTrace::new();
    trace.event("ingress");
    let guard = trace.guard();
    let response = CURRENT.scope(trace.clone(), next.run(request)).await;
    guard.finish();
    trace.response(response)
}

struct ObservedBody {
    body: Body,
    trace: MoveTrace,
    status: u16,
    complete: bool,
    failed: bool,
    bytes: u64,
}
impl HttpBody for ObservedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        let result = Pin::new(&mut self.body).poll_frame(cx);
        match &result {
            Poll::Ready(Some(Err(error))) => {
                self.failed = true;
                tracing::warn!(request_id = self.trace.0.id, error = %error, "Move response body error");
            }
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    self.bytes += data.len() as u64;
                }
                self.complete = self.body.is_end_stream();
            }
            Poll::Ready(None) => self.complete = true,
            Poll::Pending => {}
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}
impl Drop for ObservedBody {
    fn drop(&mut self) {
        let event = if self.failed {
            "response_body_error"
        } else if self.complete {
            "response_body_consumed"
        } else {
            "response_body_dropped"
        };
        let details = self.trace.0.details.lock().unwrap();
        tracing::info!(request_id = self.trace.0.id, response_bytes = self.bytes, status = self.status,
            game_id = details.game_id.as_deref().unwrap_or("unparsed"), turn = details.turn.unwrap_or(-1),
            elapsed = ?self.trace.0.started.elapsed(), event, "Move response lifecycle");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct LogWriter(Arc<Mutex<Vec<u8>>>);
    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct BrokenBody;
    impl HttpBody for BrokenBody {
        type Data = Bytes;
        type Error = std::io::Error;
        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
            Poll::Ready(Some(Err(std::io::Error::other("injected body error"))))
        }
    }

    #[tokio::test]
    async fn body_logs_distinguish_consumption_abandonment_and_errors() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(move || LogWriter(Arc::clone(&writer)))
            .finish();
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let trace = MoveTrace::new();
        trace.identify("body-game", 42);
        drop(trace.response(Response::new(Body::from("abandoned"))));
        let response = trace.response(Response::new(Body::from("consumed")));
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let response = trace.response(Response::new(Body::new(BrokenBody)));
        assert!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .is_err()
        );
        let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        for event in [
            "response_body_dropped",
            "response_body_consumed",
            "response_body_error",
        ] {
            assert!(
                logs.lines().any(|line| line.contains(event)
                    && line.contains("game_id=\"body-game\"")
                    && line.contains("turn=42")),
                "{logs}"
            );
        }
        assert!(logs.contains("response_bytes=8"));
    }
    #[tokio::test]
    async fn cancelled_future_records_last_phase_and_identity_once() {
        let trace = MoveTrace::new();
        trace.identify("cancelled-game", 244);
        let worker_trace = trace.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _outer = worker_trace.guard();
            let _inner = worker_trace.guard();
            worker_trace.phase("waiting_search");
            ready_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let details = trace.0.details.lock().unwrap();
        assert_eq!(details.outcome, "handler_cancelled");
        assert_eq!(details.phase, "waiting_search");
        assert_eq!(details.game_id.as_deref(), Some("cancelled-game"));
        assert_eq!(details.turn, Some(244));
    }

    #[tokio::test]
    async fn completed_handler_is_not_reclassified_and_body_is_preserved() {
        let trace = MoveTrace::new();
        trace.guard().finish();
        drop(trace.guard());
        assert_eq!(trace.0.details.lock().unwrap().outcome, "handler_returned");
        let response = trace.response(Response::new(Body::from("{\"move\":\"up\"}")));
        assert_eq!(
            response.headers()["x-bene-request-id"],
            trace.0.id.to_string()
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), b"{\"move\":\"up\"}");
    }
}
