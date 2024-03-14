use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::types::{build_snake_id_map, SnakeIDGettableGame};
use battlesnake_game_types::wire_representation::Game;
#[cfg(feature = "caching")]
use lib::EVAL_CACHE;
use lib::{calc_move, decode_state, GameStates};
use opentelemetry_otlp::WithExportConfig;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, instrument};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Registry;
use tracing_tree::HierarchicalLayer;
// TODO: Implement MCTS

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[instrument(skip(game_states))]
async fn get_move(State(game_states): State<GameStates>, body: String) -> Json<Value> {
    let start = std::time::Instant::now();
    info!("Got move request: {}", body);
    let cellboard = decode_state(body, game_states).unwrap();
    let chosen_move = calc_move(cellboard, 10, start).to_string();
    info!("Calculation took: {:?}", start.elapsed());
    Json(json!({"move": chosen_move}))
}

async fn info() -> Json<Value> {
    Json(json!({
        "apiversion": "1",
        "author": "Nereuxofficial",
        "color": "#FF5E5B",
        "head": "mlh-gene",
        "tail": "mlh-gene",
    }))
}

async fn end(State(game_states): State<GameStates>, body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    if game_state.you_are_winner() {
        info!("We won the game");
    } else {
        info!("We lost the game");
    }

    game_states.lock().unwrap().remove(&game_state.game.id);
    #[cfg(feature = "caching")]
    EVAL_CACHE.clear();
    Response::default()
}

async fn start(State(game_states): State<GameStates>, body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game started with {} snakes",
        game_state.get_snake_ids().len()
    );
    let snake_id_map = build_snake_id_map(&game_state);
    game_states
        .lock()
        .unwrap()
        .insert(game_state.game.id.clone(), snake_id_map);
    Response::default()
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().ok();
    let _guard = sentry::init((
        std::env::var("SENTRY_DSN").unwrap(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 0.0,
            ..Default::default()
        },
    ));

    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    let opentelemetry_layer = if let Ok(honeycomb_key) = std::env::var("HONEYCOMB_API_KEY") {
        let mut map = HashMap::<String, String>::new();
        map.insert("x-honeycomb-team".to_string(), honeycomb_key);
        map.insert("x-honeycomb-dataset".to_string(), "bene-snake".to_string());

        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .http()
                    .with_endpoint("https://api.honeycomb.io/v1/traces")
                    .with_timeout(Duration::from_secs(3))
                    .with_headers(map),
            )
            .install_batch(opentelemetry::runtime::Tokio)?;

        let opentelemetry_layer = OpenTelemetryLayer::new(tracer);

        Some(opentelemetry_layer)
    } else {
        None
    };

    let heirarchical = if opentelemetry_layer.is_none() {
        let heirarchical = HierarchicalLayer::default()
            .with_writer(std::io::stdout)
            .with_indent_lines(true)
            .with_indent_amount(2)
            .with_thread_names(true)
            .with_thread_ids(true)
            .with_verbose_exit(true)
            .with_verbose_entry(true)
            .with_targets(true);

        Some(heirarchical)
    } else {
        None
    };

    Registry::default()
        .with(heirarchical)
        .with(opentelemetry_layer)
        .with(env_filter)
        .with(sentry_tracing::layer())
        .try_init()?;

    let gamestates: GameStates = GameStates::new(Mutex::new(HashMap::new()));
    info!("Hello Snakes!");
    let app = Router::new()
        .route("/", get(info))
        .route("/move", post(get_move))
        .route("/info", get(info))
        .route("/start", post(start))
        .route("/end", post(end))
        .with_state(gamestates);
    let listener = tokio::net::TcpListener::bind(format!(
        "0.0.0.0:{}",
        std::env::var("PORT").expect("Please set the PORT environment variable")
    ))
    .await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::compact_representation::StandardCellBoard4Snakes11x11;
    use battlesnake_game_types::types::{SimulableGame, SnakeIDMap};
    use lib::Simulator;
    use simd_json::prelude::ArrayTrait;

    fn get_gamestate_with_id_map(id: &str, game: &Game) -> GameStates {
        let mut hm = HashMap::new();
        let id_map = build_snake_id_map(game);
        hm.insert(id.to_string(), id_map);
        GameStates::new(Mutex::new(hm))
    }

    #[tokio::test]
    async fn test_move_request_crash() {
        let game = r##"{"game":{"id":"27282030-676b-4e80-bd3d-ac6267dab02c","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_CSJ3RkTJgr6JDrwSyvMvFKrc","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}},{"id":"gs_TBqhVDycJ4mY6hyC4ghtM4mX","name":"Hovering Hobbs","latency":"","health":100,"body":[{"x":9,"y":1},{"x":9,"y":1},{"x":9,"y":1}],"head":{"x":9,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#da8a1a","head":"beach-puffin-special","tail":"beach-puffin-special"}},{"id":"gs_8RgMd48mMDQT8DpvYXXBFxPS","name":"ich heisse marvin","latency":"","health":100,"body":[{"x":9,"y":9},{"x":9,"y":9},{"x":9,"y":9}],"head":{"x":9,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff7043","head":"sand-worm","tail":"pixel"}},{"id":"gs_hbHqF9tDPRbxYWTTQ7mQBvvQ","name":"Spaceheater [dev]","latency":"","health":100,"body":[{"x":1,"y":1},{"x":1,"y":1},{"x":1,"y":1}],"head":{"x":1,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#ff8400","head":"workout","tail":"rocket"}}],"food":[{"x":2,"y":10},{"x":10,"y":2},{"x":10,"y":8},{"x":2,"y":0},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_CSJ3RkTJgr6JDrwSyvMvFKrc","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":1,"y":9},{"x":1,"y":9},{"x":1,"y":9}],"head":{"x":1,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#ff5e5b","head":"mlh-gene","tail":"mlh-gene"}}}"##;
        let game = serde_json::from_str::<Game>(game).unwrap();
        let game_states = get_gamestate_with_id_map("27282030-676b-4e80-bd3d-ac6267dab02c", &game);
        let snake_id_map = build_snake_id_map(&game);
        let cb: StandardCellBoard4Snakes11x11 = game.as_cell_board(&snake_id_map).unwrap();
        // TODO: Benchmark this sequential simulation vs the recursive one
        fn simulate_boards_from_simulated_boards(
            simulated_boards: Vec<StandardCellBoard4Snakes11x11>,
            snake_id_map: SnakeIDMap,
        ) -> Vec<StandardCellBoard4Snakes11x11> {
            simulated_boards
                .iter()
                .flat_map(|x| x.simulate(&Simulator {}, x.get_snake_ids().to_vec()))
                .map(|x| x.1)
                .collect()
        }
        // Generate all boards to depth 10
        let mut simulated_boards = vec![cb];
        for _ in 0..9 {
            simulated_boards =
                simulate_boards_from_simulated_boards(simulated_boards, snake_id_map.clone());
        }

        //let _chosen_move = get_move(State(game_states), game.to_string()).await;
    }
}
