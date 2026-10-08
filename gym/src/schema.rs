//! Independent, versioned wire and artifact contracts. No candidate code defines labels.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;
pub const RULES: &str = "standard-11-no-spawn-v1";
pub const ORACLE: &str = "robust-coalition-dag-v1";
pub const GENERATOR: &str = "constructive-splitmix64-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Move {
    Up,
    Down,
    Left,
    Right,
}
impl Move {
    pub const ALL: [Self; 4] = [Self::Up, Self::Down, Self::Left, Self::Right];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap()
    }
    pub fn step(self, p: Point) -> Point {
        let (x, y) = match self {
            Self::Up => (0, 1),
            Self::Down => (0, -1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        };
        Point {
            x: p.x + x,
            y: p.y + y,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    pub fn inside(self) -> bool {
        (0..11).contains(&self.x) && (0..11).contains(&self.y)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snake {
    pub id: String,
    pub name: String,
    pub health: i32,
    pub head: Point,
    pub body: Vec<Point>,
    pub length: usize,
}
impl Snake {
    pub fn new(id: &str, body: Vec<Point>, health: i32) -> Self {
        Self {
            id: id.into(),
            name: id.into(),
            health,
            head: body[0],
            length: body.len(),
            body,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub width: u32,
    pub height: u32,
    pub food: Vec<Point>,
    pub hazards: Vec<Point>,
    pub snakes: Vec<Snake>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Settings {
    pub food_spawn_chance: u32,
    pub minimum_food: u32,
    pub hazard_damage_per_turn: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ruleset {
    pub name: String,
    pub version: String,
    pub settings: Settings,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub id: String,
    pub ruleset: Ruleset,
    pub timeout: u64,
    pub map: String,
    pub source: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub game: Game,
    pub turn: u32,
    pub board: Board,
    pub you: Snake,
}
impl Request {
    pub fn new(snakes: Vec<Snake>, food: Vec<Point>, focal: &str) -> Self {
        Self {
            game: Game {
                id: "certification".into(),
                ruleset: Ruleset {
                    name: "standard".into(),
                    version: "v1.2.3".into(),
                    settings: Settings {
                        food_spawn_chance: 0,
                        minimum_food: 0,
                        hazard_damage_per_turn: 0,
                    },
                },
                timeout: 500,
                map: "standard".into(),
                source: "snake-gym".into(),
            },
            turn: 20,
            you: snakes.iter().find(|s| s.id == focal).unwrap().clone(),
            board: Board {
                width: 11,
                height: 11,
                food,
                hazards: vec![],
                snakes,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Objective {
    Survive,
    EatAndSurvive { food: Vec<Point> },
    SoleSurvivor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Success,
    Failure,
    Unknown,
}
/// Explicit wire direction keys, independent of compact move indexing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Labels {
    pub up: Label,
    pub down: Label,
    pub left: Label,
    pub right: Label,
}
impl Labels {
    pub fn from_array(a: [Label; 4]) -> Self {
        Self {
            up: a[0],
            down: a[1],
            left: a[2],
            right: a[3],
        }
    }
    pub fn array(&self) -> [Label; 4] {
        [self.up, self.down, self.left, self.right]
    }
    pub fn get(&self, m: Move) -> Label {
        self.array()[m.index()]
    }
    pub fn accepted(&self) -> bool {
        let a = self.array();
        !a.contains(&Label::Unknown) && a.contains(&Label::Success) && a.contains(&Label::Failure)
    }
    pub fn successes(&self) -> Vec<Move> {
        Move::ALL
            .into_iter()
            .filter(|m| self.get(*m) == Label::Success)
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub schema: u32,
    pub id: String,
    pub cluster: String,
    pub family: String,
    pub family_version: u32,
    pub seed: u64,
    pub parameters: BTreeMap<String, i64>,
    pub request: Request,
    pub objective: Objective,
    pub horizon: u32,
    pub rules_hash: String,
    pub oracle_hash: String,
    pub labels: Labels,
    pub certificate_hash: String,
    pub provenance: String,
    pub nodes: usize,
    pub minimum_failure_delay: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub schema: u32,
    pub name: String,
    pub families: Vec<String>,
    pub max_attempts_per_case: usize,
    pub node_limit: usize,
    pub certificate_limit: usize,
    pub wall_ms: u64,
    pub max_trivial_share: f64,
    pub threshold: f64,
    pub bootstrap_samples: usize,
    pub min_clusters_per_family: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyGeneration {
    pub accepted: usize,
    pub attempts: usize,
    pub rejections: BTreeMap<String, usize>,
    pub nodes: usize,
    pub elapsed_ms: u64,
    pub horizons: BTreeMap<u32, usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub schema: u32,
    pub seed: u64,
    pub requested: usize,
    pub complete: bool,
    pub suite: Suite,
    pub generator_hash: String,
    pub rules_hash: String,
    pub oracle_hash: String,
    pub corpus_hash: String,
    pub families: BTreeMap<String, FamilyGeneration>,
    pub elapsed_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub descriptor: String,
    pub sha: Option<String>,
    pub source_hash: Option<String>,
    pub lock_hash: Option<String>,
    pub binary_hash: String,
    pub binary: String,
    pub toolchain: String,
    pub flags: Vec<String>,
    pub build_log: Option<String>,
    pub submodules: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub run_id: String,
    pub created: String,
    pub corpus_hash: String,
    pub generation_hash: String,
    pub harness_hash: String,
    pub rules_hash: String,
    pub oracle_hash: String,
    pub candidates: [Candidate; 2],
    pub repeats: usize,
    pub timeout_ms: u64,
    pub suite: Suite,
    pub machine: String,
    pub mode: String,
    pub environment: BTreeMap<String, String>,
    pub build_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Valid,
    Fatal,
    Malformed,
    Timeout,
    HttpError,
    Crash,
    StartupFailure,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub schema: u32,
    pub run_id: String,
    pub case_id: String,
    pub candidate: usize,
    pub repeat: usize,
    pub order: usize,
    pub request_hash: String,
    pub binary_hash: String,
    pub status: Status,
    pub chosen: Option<Move>,
    pub response: Option<serde_json::Value>,
    pub latency_ms: f64,
    pub startup_ms: f64,
    pub deadline_ms: u64,
    pub timestamp: String,
    pub log: String,
    pub detail: String,
}
