//! Replay recorded Arena positions through the engine and report the chosen move.
//!
//! Deterministic: a fixed RNG seed and a fixed iteration count, so building this at two
//! commits shows whether a change moved any recorded decision. With a turn argument it
//! plays in closed loop against the *recorded* opponent moves instead.
use std::{path::Path, sync::Arc, time::Instant};

use battlesnake_game_types::{
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
        ReasonableMovesGame, SimulableGame, SnakeId, YouDeterminableGame, build_snake_id_map,
    },
    wire_representation::{BattleSnake, Game, Position},
};
use lib::mcts::{Node, SearchDepthStats, search_once_with_rng};
use rand::SeedableRng;
use serde::Deserialize;
use serde_json::{Value, json};

/// Replay coordinates are PascalCase, the wire format is lowercase.
#[derive(Deserialize, Clone, Copy)]
struct Point {
    #[serde(rename = "X")]
    x: i32,
    #[serde(rename = "Y")]
    y: i32,
}

impl From<Point> for Position {
    fn from(point: Point) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }
}

/// The name this engine registers with on the Arena.
const OWN_NAME: &str = "bene-snake";

#[derive(Deserialize, Clone)]
struct Death {
    #[serde(rename = "Cause")]
    cause: String,
    #[serde(rename = "Turn")]
    turn: i64,
}

#[derive(Deserialize)]
struct ReplaySnake {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Health")]
    health: i32,
    #[serde(rename = "Body")]
    body: Vec<Point>,
    #[serde(rename = "Death")]
    death: Option<Death>,
}

#[derive(Deserialize)]
struct RawFrame {
    #[serde(rename = "Turn")]
    turn: i32,
    #[serde(rename = "Snakes")]
    snakes: Vec<ReplaySnake>,
    #[serde(rename = "Food")]
    food: Vec<Point>,
    #[serde(default, rename = "Hazards")]
    hazards: Option<Vec<Point>>,
}

/// One recorded turn, ready to be handed to the engine.
struct Recorded {
    game: Game,
    /// The move the arena actually applied on the next frame.
    applied: Option<&'static str>,
    death: Option<Death>,
}

fn to_wire(frame: &RawFrame, alive: &[String], game_id: &str, us: &str) -> Option<Game> {
    // The wire format's `you` is the first entry of `board.snakes`, so put our own snake
    // first. Otherwise the probe silently plays whichever snake the arena happened to
    // list first, which is not the one whose recorded moves we are comparing.
    let mut ours: Vec<BattleSnake> = Vec::new();
    let mut others: Vec<BattleSnake> = Vec::new();
    for snake in frame.snakes.iter().filter(|s| alive.contains(&s.id)) {
        let converted = BattleSnake {
            id: snake.id.clone(),
            name: snake.name.clone(),
            head: snake.body[0].into(),
            body: snake.body.iter().map(|p| (*p).into()).collect(),
            health: snake.health,
            shout: None,
            actual_length: None,
        };
        if snake.id == us {
            ours.push(converted);
        } else {
            others.push(converted);
        }
    }
    if ours.is_empty() {
        return None;
    }
    let snakes: Vec<BattleSnake> = ours.into_iter().chain(others).collect();
    let game: Game = serde_json::from_value(json!({
        "game": {
            "id": game_id,
            "ruleset": { "name": "standard", "version": "1" },
            "timeout": 500,
        },
        "turn": frame.turn,
        "board": {
            "height": 11,
            "width": 11,
            "snakes": snakes,
            "food": frame
                .food
                .iter()
                .map(|p| json!({ "x": p.x, "y": p.y }))
                .collect::<Vec<_>>(),
            "hazards": frame
                .hazards
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|p| json!({ "x": p.x, "y": p.y }))
                .collect::<Vec<_>>(),
        },
        "you": snakes[0],
    }))
    .expect("recorded frame is a valid wire request");
    Some(game)
}

/// Find our own snake by name, the same snake the server was playing as.
fn own_id(frames: &[RawFrame]) -> Option<String> {
    frames
        .first()?
        .snakes
        .iter()
        .find(|snake| snake.name == OWN_NAME)
        .map(|snake| snake.id.clone())
}

fn load(path: &Path) -> Vec<Recorded> {
    let text = std::fs::read_to_string(path).expect("read replay");
    let raw: Value = serde_json::from_str(&text).expect("parse replay JSON");
    let frames: Vec<RawFrame> =
        serde_json::from_value(raw["frames"].clone()).expect("parse frames");
    let game_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("replay")
        .to_owned();

    // The arena keeps dead snakes in later frames until the game ends, so track liveness
    // the way the server's normalization does.
    let own = own_id(&frames).expect("replay contains our own snake");
    let mut alive: Vec<String> = frames
        .first()
        .map(|frame| frame.snakes.iter().map(|s| s.id.clone()).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let next = frames.get(index + 1);
        let applied = next.and_then(|next| {
            let here = frame.snakes.iter().find(|s| s.id == own)?.body[0];
            let there = next.snakes.iter().find(|s| s.id == own)?.body[0];
            let delta = (there.x - here.x, there.y - here.y);
            match delta {
                (1, 0) => Some("right"),
                (-1, 0) => Some("left"),
                (0, 1) => Some("up"),
                (0, -1) => Some("down"),
                _ => None,
            }
        });
        let death = next.and_then(|next| {
            next.snakes
                .iter()
                .find(|s| s.id == own)
                .and_then(|s| s.death.clone())
        });
        if let Some(game) = to_wire(frame, &alive, &game_id, &own) {
            out.push(Recorded {
                game,
                applied,
                death,
            });
        }
        if let Some(next) = next {
            alive.retain(|id| {
                next.snakes
                    .iter()
                    .find(|s| &s.id == id)
                    .is_some_and(|s| s.death.is_none())
            });
            if alive.is_empty() {
                break;
            }
        }
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: replay_probe <replay.json> [iterations] [turn]");
    let iterations: u64 = args.next().map_or(20_000, |v| v.parse().unwrap());
    let only: Option<i32> = args.next().map(|v| v.parse().unwrap());
    let recorded = load(Path::new(&path));
    if let Some(start) = only {
        return selfplay(&recorded, start, iterations);
    }

    println!("# turn applied chosen agree | candidate weights (up down left right)");
    for record in recorded {
        let ids = build_snake_id_map(&record.game);
        let Ok(board) = record.game.as_cell_board(&ids) else {
            continue;
        };
        let you = *board.you_id();
        let root = Arc::new(Node::new_root(board));
        let (mask, weights) = root.root_candidates(you).expect("our snake has candidates");
        let start = Instant::now();
        choose(&root, you, iterations);
        let chosen = root.best_move(you).map(move_name).map(str::to_owned);
        let agree = match (&record.applied, &chosen) {
            (Some(applied), Some(chosen)) if applied == chosen => "same".to_owned(),
            (Some(_), Some(_)) => "CHANGED".to_owned(),
            _ => "-".to_owned(),
        };
        let names = ["up", "down", "left", "right"];
        let weights = names
            .iter()
            .enumerate()
            .map(|(i, mv)| {
                if mask >> i & 1 == 0 {
                    format!("{mv}=-")
                } else {
                    format!("{mv}={}", weights[i])
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let death = record
            .death
            .as_ref()
            .map(|d| format!("# death={} t={}", d.cause, d.turn))
            .unwrap_or_default();
        println!(
            "{} {} {} {agree} | {weights} | {}ms {death}",
            record.game.turn,
            record.applied.unwrap_or("-"),
            chosen.as_deref().unwrap_or("-"),
            start.elapsed().as_millis()
        );
    }
}

/// Bounded search on a fixed seed, so two builds are directly comparable.
fn choose(root: &Arc<Node>, you: SnakeId, iterations: u64) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5EED);
    let mut stats = SearchDepthStats::default();
    for _ in 0..iterations {
        search_once_with_rng(root, &you, &mut stats, &mut rng);
    }
}

/// Closed loop from `start`: our engine against the *recorded* opponent moves, so any
/// divergence is ours.
fn selfplay(recorded: &[Recorded], start: i32, iterations: u64) {
    let Some(index) = recorded.iter().position(|r| r.game.turn == start) else {
        eprintln!("no recorded turn {start}");
        return;
    };
    let mut game = recorded[index].game.clone();

    let limit = start + 60;
    println!("# closed-loop replay from turn {start} (opponent follows the recording)");
    println!("turn chosen result");
    for turn in start..limit {
        let ids = build_snake_id_map(&game);
        let Ok(board) = game.as_cell_board(&ids) else {
            break;
        };
        let us = ids[&game.you.id];
        let root = Arc::new(Node::new_root(board));
        choose(&root, us, iterations);
        let Some(chosen) = root.best_move(us) else {
            println!("{turn} - no legal move");
            return;
        };

        // Opponents replay their recorded move for this turn, falling back to anything legal.
        let mut moves: Vec<(SnakeId, [Move; 1])> = vec![(us, [chosen])];
        for snake in &game.board.snakes {
            if snake.id == game.you.id {
                continue;
            }
            let id = ids[&snake.id];
            if board.get_health(&id) == 0 {
                continue;
            }
            let recorded_move = recorded
                .iter()
                .find(|r| r.game.turn == turn)
                .and_then(|r| r.applied)
                .and_then(name_to_move);
            let mask = board
                .reasonable_moves_for_each_snake()
                .into_iter()
                .find(|(other, _)| *other == id)
                .map(|(_, moves)| moves)
                .unwrap_or_default();
            let pick = recorded_move
                .filter(|mv| mask.contains(mv))
                .or_else(|| mask.first().copied());
            if let Some(mv) = pick {
                moves.push((id, [mv]));
            }
        }
        let observed = board
            .simulate_with_moves(&moves)
            .last()
            .map(|(_, board)| board);
        let Some(next) = observed else {
            println!("{turn} {} simulation failed", move_name(chosen));
            return;
        };
        if next.get_health(&us) == 0 {
            println!("{turn} {} DIED", move_name(chosen));
            return;
        }
        println!("{turn} {} survived", move_name(chosen));
        game.turn += 1;
        game.board.food = next.get_all_food_as_positions().into_iter().collect();
        for snake in game.board.snakes.iter_mut() {
            let id = ids[&snake.id];
            if next.get_health(&id) == 0 {
                snake.health = 0;
                continue;
            }
            let head = next.get_head_as_position(&id);
            let grew = next.get_length(&id) as usize > snake.body.len();
            snake.health = next.get_health_i64(&id) as i32;
            snake.head = head;
            snake.body.push_front(head);
            if !grew {
                snake.body.pop_back();
            }
        }
    }
    println!("(survived to turn {limit})");
}

fn move_name(mv: Move) -> &'static str {
    match mv {
        Move::Up => "up",
        Move::Down => "down",
        Move::Left => "left",
        Move::Right => "right",
    }
}

fn name_to_move(name: &str) -> Option<Move> {
    match name {
        "up" => Some(Move::Up),
        "down" => Some(Move::Down),
        "left" => Some(Move::Left),
        "right" => Some(Move::Right),
        _ => None,
    }
}
