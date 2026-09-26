//! Compare length-weight variants of the evaluation function against held-out
//! game outcomes using replay positions. Games, rather than turns, are the
//! statistical unit: each eligible game contributes one mean score per variant.
//!
//! Run from the workspace root with:
//! `cargo run --release -p lib --example evaluation_score_benchmark --`

use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{SnakeId, build_snake_id_map},
    wire_representation::Game,
};
use lib::eval::evaluate_board_with_length_weight;
use serde_json::{Value, json};

const DEFAULT_INPUT: &str = "experiments/nereuxofficial-positions.jsonl";
const DEFAULT_OUTPUT: &str = "experiments/evaluation-score-benchmark.csv";
const DEFAULT_WEIGHTS: &[i32] = &[0, 5, 10, 15, 20, 30, 45];

struct Options {
    input: PathBuf,
    output: PathBuf,
    stride: i32,
    weights: Vec<i32>,
}

#[derive(Default)]
struct GameScores {
    won: bool,
    sums: Vec<u64>,
    count: u64,
}

fn options() -> Result<Options, String> {
    let mut input = PathBuf::from(DEFAULT_INPUT);
    let mut output = PathBuf::from(DEFAULT_OUTPUT);
    let mut stride = 5;
    let mut weights = DEFAULT_WEIGHTS.to_vec();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = || format!("missing value for {arg}");
        match arg.as_str() {
            "--input" => input = args.next().ok_or_else(value)?.into(),
            "--output" => output = args.next().ok_or_else(value)?.into(),
            "--stride" => {
                stride = args
                    .next()
                    .ok_or_else(value)?
                    .parse()
                    .map_err(|_| "invalid --stride".to_string())?;
                if stride < 1 {
                    return Err("--stride must be positive".into());
                }
            }
            "--length-weights" => {
                weights = args
                    .next()
                    .ok_or_else(value)?
                    .split(',')
                    .map(|weight| {
                        weight
                            .parse()
                            .map_err(|_| format!("invalid length weight: {weight}"))
                    })
                    .collect::<Result<_, _>>()?;
                if weights.is_empty() {
                    return Err("--length-weights cannot be empty".into());
                }
            }
            "--help" | "-h" => {
                println!(
                    "Usage: evaluation_score_benchmark [--input JSONL] [--output CSV] [--stride N] [--length-weights CSV]\n\nEach game contributes the mean score of its live positions sampled every N turns. The CSV reports how well those per-game means separate eventual wins from eliminations."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(Options {
        input,
        output,
        stride,
        weights,
    })
}

fn coord_list(value: &Value) -> Result<Vec<Value>, String> {
    value
        .as_array()
        .ok_or("expected coordinate array".to_string())?
        .iter()
        .map(|p| {
            Ok(json!({"x": p["X"].as_i64().ok_or("missing X coordinate")?,
                  "y": p["Y"].as_i64().ok_or("missing Y coordinate")?}))
        })
        .collect()
}

fn game_from_position(record: &Value) -> Result<Option<(Game, SnakeId)>, String> {
    let frame = &record["frame"];
    let info = &record["game_info"];
    let raw_snakes = frame["Snakes"].as_array().ok_or("frame has no Snakes")?;
    let last_snakes = info["LastFrame"]["Snakes"]
        .as_array()
        .ok_or("game metadata has no snakes")?;
    let you_id = last_snakes
        .iter()
        .find(|snake| {
            snake["Author"]
                .as_str()
                .is_some_and(|author| author.eq_ignore_ascii_case("Nereuxofficial"))
        })
        .or_else(|| {
            last_snakes
                .iter()
                .find(|snake| snake["Name"] == "bene-snake")
        })
        .and_then(|snake| snake["ID"].as_str())
        .ok_or("could not identify bene-snake in game metadata")?;

    let mut snakes = Vec::new();
    for snake in raw_snakes {
        if !snake["Death"].is_null() {
            continue;
        }
        let body = coord_list(&snake["Body"])?;
        let head = body.first().ok_or("live snake has empty body")?.clone();
        snakes.push(json!({
            "id": snake["ID"], "name": snake["Name"], "head": head,
            "body": body, "health": snake["Health"], "shout": snake["Shout"]
        }));
    }
    let Some(you) = snakes.iter().find(|snake| snake["id"] == you_id).cloned() else {
        return Ok(None);
    };
    let food = coord_list(&frame["Food"])?;
    let hazards = coord_list(&frame["Hazards"])?;
    let state = json!({
        "you": you,
        "board": {
            "height": info["Game"]["Height"], "width": info["Game"]["Width"],
            "food": food, "hazards": hazards, "snakes": snakes
        },
        "turn": frame["Turn"],
        "game": {"id": record["game_id"], "ruleset": {"name": "standard", "version": "v1", "settings": null}, "timeout": 500},
        "timeout": 500
    });
    let game: Game = serde_json::from_value(state)
        .map_err(|error| format!("invalid replay position: {error}"))?;
    let ids = build_snake_id_map(&game);
    let you = *ids
        .get(you_id)
        .ok_or("bene-snake missing from snake ID map")?;
    Ok(Some((game, you)))
}

fn auc(scores: &[(f64, bool)]) -> Option<f64> {
    let wins: Vec<f64> = scores
        .iter()
        .filter_map(|(score, won)| won.then_some(*score))
        .collect();
    let losses: Vec<f64> = scores
        .iter()
        .filter_map(|(score, won)| (!won).then_some(*score))
        .collect();
    if wins.is_empty() || losses.is_empty() {
        return None;
    }
    let mut wins_over_losses = 0.0;
    for win in &wins {
        for loss in &losses {
            wins_over_losses += if win > loss {
                1.0
            } else if win == loss {
                0.5
            } else {
                0.0
            };
        }
    }
    Some(wins_over_losses / (wins.len() * losses.len()) as f64)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options().map_err(std::io::Error::other)?;
    let mut games: BTreeMap<String, GameScores> = BTreeMap::new();
    let reader = BufReader::new(File::open(&options.input)?);
    let mut positions_scored = 0usize;
    for (line_index, line) in reader.lines().enumerate() {
        let line = line?;
        let record: Value = serde_json::from_str(&line)
            .map_err(|error| format!("invalid JSON at line {}: {error}", line_index + 1))?;
        let turn = record["frame"]["Turn"]
            .as_i64()
            .ok_or("frame has no Turn")? as i32;
        if turn % options.stride != 0 {
            continue;
        }
        let Some((game, you)) = game_from_position(&record)? else {
            continue;
        };
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&build_snake_id_map(&game))?;
        let won = record["profile_result"].as_str() == Some("Winner!");
        let entry = games
            .entry(
                record["game_id"]
                    .as_str()
                    .ok_or("record has no game_id")?
                    .to_owned(),
            )
            .or_insert_with(|| GameScores {
                won,
                sums: vec![0; options.weights.len()],
                count: 0,
            });
        for (sum, weight) in entry.sums.iter_mut().zip(&options.weights) {
            *sum += evaluate_board_with_length_weight(&board, &you, *weight) as u64;
        }
        entry.count += 1;
        positions_scored += 1;
    }

    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = File::create(&options.output)?;
    use std::io::Write;
    let game_rows: Vec<_> = games.iter().collect();
    let train_games = game_rows.len() * 7 / 10;
    writeln!(
        output,
        "split,length_weight,games,wins,eliminations,sampled_positions,game_weighted_auc,mean_win_score,mean_eliminated_score"
    )?;
    for (index, weight) in options.weights.iter().enumerate() {
        for (split, rows) in [
            ("train", &game_rows[..train_games]),
            ("test", &game_rows[train_games..]),
        ] {
            let results: Vec<(f64, bool)> = rows
                .iter()
                .map(|(_, game)| (game.sums[index] as f64 / game.count as f64, game.won))
                .collect();
            let wins = results.iter().filter(|(_, won)| *won).count();
            let losses = results.len() - wins;
            let sampled_positions: u64 = rows.iter().map(|(_, game)| game.count).sum();
            let mean = |won: bool| {
                let values: Vec<f64> = results
                    .iter()
                    .filter_map(|(score, result)| (*result == won).then_some(*score))
                    .collect();
                if values.is_empty() {
                    0.0
                } else {
                    values.iter().sum::<f64>() / values.len() as f64
                }
            };
            let auc = auc(&results)
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default();
            writeln!(
                output,
                "{split},{weight},{},{wins},{losses},{sampled_positions},{auc},{:.3},{:.3}",
                results.len(),
                mean(true),
                mean(false)
            )?;
            println!(
                "length_weight={weight:>2} {split:>5} games={} wins={wins} eliminations={losses} AUC={}",
                results.len(),
                if auc.is_empty() { "n/a" } else { &auc }
            );
        }
    }
    println!(
        "Scored {positions_scored} live positions at stride {} from {} games (UUID-sorted 70/30 game split); wrote {}",
        options.stride,
        games.len(),
        options.output.display()
    );
    Ok(())
}
