//! Optional live game recording and a self-contained browser replay viewer.
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    runner::{GameConfig, run_game_observed},
    stats::GameResult,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame,
        PositionGettableGame, SnakeBodyGettableGame, SnakeId,
    },
    wire_representation::Position,
};
use lib::Agent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnakeFrame {
    pub seat: usize,
    pub health: u8,
    pub length: u16,
    pub head: Position,
    pub body: Vec<Position>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub turn: u32,
    pub food: Vec<Position>,
    /// Only living snakes have bodies on the compact board.
    pub snakes: Vec<SnakeFrame>,
}

impl Frame {
    fn capture(turn: u32, board: &CellBoard4Snakes11x11, num_snakes: usize) -> Self {
        let snakes = (0..num_snakes)
            .filter_map(|seat| {
                let id = SnakeId(seat as u8);
                if !board.is_alive(&id) {
                    return None;
                }
                Some(SnakeFrame {
                    seat,
                    health: board.get_health(&id),
                    length: board.get_length(&id),
                    head: board.get_head_as_position(&id),
                    body: board
                        .get_snake_body_vec(&id)
                        .into_iter()
                        .map(|pos| board.position_from_native(pos))
                        .collect(),
                })
            })
            .collect();
        Self {
            turn,
            food: board.get_all_food_as_positions().into_iter().collect(),
            snakes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameSummary {
    pub id: String,
    pub started_at: u64,
    /// Names are in board seat order (including a duel's swapped seats).
    pub agents: Vec<String>,
    pub seed: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub max_turns: u32,
    pub turns: u32,
    pub status: String,
    /// Winner is a board seat, before the CLI's aggregate winner remapping.
    pub result: Option<GameResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Replay {
    summary: GameSummary,
    frames: Vec<Frame>,
}

enum Entry {
    Live(Arc<RwLock<Replay>>),
    Saved(GameSummary),
}

/// Completed replays stay on disk; only running games retain frames in memory.
/// Each live game has its own lock so parallel games can publish independently.
pub struct ReplayStore {
    directory: PathBuf,
    entries: Mutex<BTreeMap<String, Entry>>,
    prefix: String,
    next_id: AtomicU64,
    turn_delay: Duration,
}

impl ReplayStore {
    pub fn open(directory: PathBuf, turn_delay: Duration) -> io::Result<Arc<Self>> {
        std::fs::create_dir_all(&directory)?;
        let mut entries = BTreeMap::new();
        for file in std::fs::read_dir(&directory)? {
            let path = file?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            match std::fs::read(&path).and_then(|bytes| {
                serde_json::from_slice::<Replay>(&bytes).map_err(io::Error::other)
            }) {
                Ok(replay)
                    if path.file_stem().and_then(|name| name.to_str())
                        == Some(&replay.summary.id)
                        && replay.summary.status == "complete"
                        && !replay.frames.is_empty() =>
                {
                    entries.insert(replay.summary.id.clone(), Entry::Saved(replay.summary));
                }
                Ok(_) => eprintln!("Skipping invalid gym replay {}", path.display()),
                Err(error) => eprintln!("Skipping gym replay {}: {error}", path.display()),
            }
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Ok(Arc::new(Self {
            directory,
            entries: Mutex::new(entries),
            prefix: format!("{}-{}", timestamp.as_nanos(), std::process::id()),
            next_id: AtomicU64::new(0),
            turn_delay,
        }))
    }

    pub fn run_game(
        &self,
        agents: &[&dyn Agent],
        config: &GameConfig,
        seed: Option<u64>,
    ) -> GameResult {
        let id = format!(
            "{}-{}",
            self.prefix,
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let replay = Arc::new(RwLock::new(Replay {
            summary: GameSummary {
                id: id.clone(),
                started_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                agents: agents
                    .iter()
                    .take(config.num_snakes)
                    .map(|agent| agent.name().to_string())
                    .collect(),
                seed,
                width: config.width,
                height: config.height,
                max_turns: config.max_turns,
                turns: 0,
                status: "running".into(),
                result: None,
            },
            frames: Vec::new(),
        }));
        self.entries
            .lock()
            .unwrap()
            .insert(id.clone(), Entry::Live(Arc::clone(&replay)));
        let result = run_game_observed(agents, config, seed, |turn, board| {
            let frame = Frame::capture(turn, board, config.num_snakes);
            {
                let mut replay = replay.write().unwrap();
                replay.summary.turns = turn;
                replay.frames.push(frame);
            }
            if !self.turn_delay.is_zero() {
                std::thread::sleep(self.turn_delay);
            }
        });
        let summary = {
            let mut replay = replay.write().unwrap();
            replay.summary.status = "complete".into();
            replay.summary.result = Some(result.clone());
            replay.summary.clone()
        };
        // Write outside the index lock, then atomically expose the finished file.
        let save = (|| -> io::Result<()> {
            let bytes = serde_json::to_vec(&*replay.read().unwrap()).map_err(io::Error::other)?;
            let temporary = self.directory.join(format!("{id}.tmp"));
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(temporary, self.directory.join(format!("{id}.json")))
        })();
        match save {
            Ok(()) => {
                self.entries
                    .lock()
                    .unwrap()
                    .insert(id, Entry::Saved(summary));
            }
            Err(error) => eprintln!("Could not save gym replay {id}: {error}"),
        }
        result
    }

    pub fn summaries(&self) -> Vec<GameSummary> {
        let mut summaries: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|entry| match entry {
                Entry::Live(replay) => replay.read().unwrap().summary.clone(),
                Entry::Saved(summary) => summary.clone(),
            })
            .collect();
        summaries.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        summaries
    }

    fn frames(&self, id: &str, from: usize) -> Result<FrameResponse, StatusCode> {
        // Resolve from the index first; arbitrary URL paths are never file paths.
        let live = match self.entries.lock().unwrap().get(id) {
            Some(Entry::Live(replay)) => Some(Arc::clone(replay)),
            Some(Entry::Saved(_)) => None,
            None => return Err(StatusCode::NOT_FOUND),
        };
        let subset = |replay: &Replay| {
            let from = from.min(replay.frames.len());
            FrameResponse {
                summary: replay.summary.clone(),
                from,
                frames: replay.frames[from..].to_vec(),
            }
        };
        if let Some(replay) = live {
            return Ok(subset(&replay.read().unwrap()));
        }
        let bytes = std::fs::read(self.directory.join(format!("{id}.json")))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let replay: Replay =
            serde_json::from_slice(&bytes).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(subset(&replay))
    }
}

#[derive(Serialize)]
struct FrameResponse {
    summary: GameSummary,
    from: usize,
    frames: Vec<Frame>,
}

#[derive(Default, Deserialize)]
struct FrameQuery {
    #[serde(default)]
    from: usize,
}

pub fn router(store: Arc<ReplayStore>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(include_str!("viewer.html")) }))
        .route(
            "/api/games",
            get(|State(store): State<Arc<ReplayStore>>| async move { Json(store.summaries()) }),
        )
        .route("/api/games/{id}", get(game_frames))
        .route("/api/games/{id}/replay", get(download_replay))
        .with_state(store)
}

async fn game_frames(
    State(store): State<Arc<ReplayStore>>,
    Path(id): Path<String>,
    Query(query): Query<FrameQuery>,
) -> Result<Json<FrameResponse>, StatusCode> {
    tokio::task::spawn_blocking(move || store.frames(&id, query.from))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
}

async fn download_replay(
    State(store): State<Arc<ReplayStore>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let data = tokio::task::spawn_blocking(move || store.frames(&id, 0))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    Ok((
        [(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"gym-replay.json\"",
        )],
        Json(Replay {
            summary: data.summary,
            frames: data.frames,
        }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{HeuristicAgent, HeuristicPolicy};

    #[test]
    fn parallel_games_publish_live_frames_before_finishing() {
        use battlesnake_game_types::types::{Move, SnakeId};
        use std::sync::Barrier;
        struct Gate {
            ready: Arc<Barrier>,
            release: Arc<Barrier>,
        }
        impl Agent for Gate {
            fn name(&self) -> &str {
                "gated-agent"
            }
            fn choose_move(&self, _board: &CellBoard4Snakes11x11, _you: SnakeId) -> Move {
                self.ready.wait();
                self.release.wait();
                Move::Up
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "gym-live-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = ReplayStore::open(directory.clone(), Duration::ZERO).unwrap();
        let ready = Arc::new(Barrier::new(3));
        let release = Arc::new(Barrier::new(3));
        std::thread::scope(|scope| {
            for seed in [1234, 1235] {
                let store = Arc::clone(&store);
                let gate = Gate {
                    ready: Arc::clone(&ready),
                    release: Arc::clone(&release),
                };
                scope.spawn(move || {
                    let other = HeuristicAgent::new();
                    let config = GameConfig {
                        max_turns: 1,
                        ..GameConfig::duel()
                    };
                    store.run_game(&[&gate, &other], &config, Some(seed));
                });
            }
            ready.wait();
            let live = store.summaries();
            assert_eq!(live.len(), 2);
            assert_ne!(live[0].id, live[1].id);
            for summary in live {
                assert_eq!(summary.status, "running");
                let frames = store.frames(&summary.id, 0).unwrap();
                assert_eq!(frames.frames.len(), 1);
                assert_eq!(frames.frames[0].snakes.len(), 2);
                assert!(frames.summary.result.is_none());
            }
            release.wait();
        });
        for summary in store.summaries() {
            assert_eq!(summary.status, "complete");
            assert_eq!(store.frames(&summary.id, 0).unwrap().frames.len(), 2);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn records_complete_game_and_reopens_history_without_changing_result() {
        let directory = std::env::temp_dir().join(format!(
            "gym-viewer-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = ReplayStore::open(directory.clone(), Duration::ZERO).unwrap();
        let agents = [
            HeuristicAgent::with_policy("legacy", HeuristicPolicy::Legacy),
            HeuristicAgent::new(),
        ];
        let refs: Vec<&dyn Agent> = agents.iter().map(|a| a as &dyn Agent).collect();
        let config = GameConfig {
            max_turns: 150,
            ..GameConfig::duel()
        };
        let expected = crate::runner::run_game_seeded(&refs, &config, 1234);
        let actual = store.run_game(&refs, &config, Some(1234));
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            serde_json::to_value(&actual).unwrap()
        );
        assert_eq!(
            actual.winner,
            Some(1),
            "replay winner must retain board seat order"
        );
        let summary = store.summaries().remove(0);
        let replay = store.frames(&summary.id, 0).unwrap();
        assert_eq!(replay.frames.len(), actual.turns as usize + 1);
        assert_eq!(replay.frames[0].turn, 0);
        assert_eq!(replay.frames.last().unwrap().turn, actual.turns);
        assert_eq!(replay.summary.status, "complete");
        assert_eq!(replay.frames.last().unwrap().snakes.len(), 1);
        assert_eq!(replay.frames.last().unwrap().snakes[0].seat, 1);
        for frame in &replay.frames {
            for snake in &frame.snakes {
                assert_eq!(snake.body[0], snake.head);
                assert_eq!(snake.body.len(), usize::from(snake.length));
                assert!(snake.health > 0);
            }
        }
        assert_eq!(
            store.frames(&summary.id, 2).unwrap().frames.len(),
            replay.frames.len().saturating_sub(2)
        );
        assert!(
            store
                .frames(&summary.id, usize::MAX)
                .unwrap()
                .frames
                .is_empty()
        );
        assert!(matches!(
            store.frames("../outside", 0),
            Err(StatusCode::NOT_FOUND)
        ));
        drop(store);
        let reopened = ReplayStore::open(directory.clone(), Duration::ZERO).unwrap();
        assert_eq!(reopened.summaries().len(), 1);
        assert_eq!(
            reopened.frames(&summary.id, 0).unwrap().frames.len(),
            replay.frames.len()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
