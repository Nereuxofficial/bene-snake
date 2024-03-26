use serde_json::Value;
use tracing::debug;

#[derive(Debug, serde::Deserialize)]
struct EnginePosition {
    #[serde(rename = "X")]
    x: usize,
    #[serde(rename = "Y")]
    y: usize,
}

#[derive(Debug, serde::Deserialize)]
struct Snake {
    #[serde(rename = "Body")]
    body: Vec<EnginePosition>,
}

#[derive(Debug, serde::Deserialize)]
struct Frame {
    #[serde(rename = "Food")]
    food: Vec<EnginePosition>,
    #[serde(rename = "Snakes")]
    snakes: Vec<Snake>,
    #[serde(rename = "Turn")]
    turn: usize,
}

#[derive(Debug, serde::Deserialize)]
struct EngineResponse {
    #[serde(rename = "Count")]
    count: usize,
    #[serde(rename = "Frames")]
    frames: Vec<Frame>,
}

#[tokio::main]
async fn main() {
    let mut client = reqwest::Client::new();
    let game_id = "f32a0176-85a7-4b6d-a9de-bb68c7fd7f21";
    let res: EngineResponse = request_game_frame(&client, game_id).await.unwrap();
    println!("{:?}", res);
}

async fn request_game_frame(
    client: &reqwest::Client,
    game_id: &str,
) -> Result<EngineResponse, reqwest::Error> {
    // TODO: Only returns max of 100 frames, we need to handle pagination
    let response = client
        .get(format!(
            "https://engine.battlesnake.com/games/{}/frames?offset={}&limit=1000",
            game_id, 0
        ))
        .send()
        .await?;
    let response_text = response.text().await?;
    debug!("Response: {:?}", response_text);
    let response_json: EngineResponse = serde_json::from_str(&response_text).unwrap();
    Ok(response_json)
}
