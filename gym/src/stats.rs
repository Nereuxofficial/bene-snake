use serde::{Deserialize, Serialize};

/// Result of a single game
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameResult {
    /// Index of the winning agent, or None if it was a draw
    pub winner: Option<usize>,
    /// Number of turns the game lasted
    pub turns: u32,
    /// Number of snakes in the game
    pub num_snakes: usize,
}

/// Aggregated statistics for an agent
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentStats {
    pub name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub total_games: u32,
    pub total_turns: u64,
}

impl AgentStats {
    pub fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.total_games == 0 {
            0.0
        } else {
            self.wins as f64 / self.total_games as f64
        }
    }

    pub fn avg_game_length(&self) -> f64 {
        if self.total_games == 0 {
            0.0
        } else {
            self.total_turns as f64 / self.total_games as f64
        }
    }
}

/// Tournament statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TournamentStats {
    pub agent_stats: Vec<AgentStats>,
    pub total_games: u32,
    pub total_draws: u32,
    pub avg_game_length: f64,
    pub min_game_length: u32,
    pub max_game_length: u32,
}

impl TournamentStats {
    /// Compute statistics from game results
    pub fn from_results(results: &[GameResult], agent_names: &[String]) -> Self {
        let mut agent_stats: Vec<AgentStats> = agent_names
            .iter()
            .map(|name| AgentStats::new(name.clone()))
            .collect();

        let mut total_draws = 0u32;
        let mut min_length = u32::MAX;
        let mut max_length = 0u32;
        let mut total_turns = 0u64;

        for result in results {
            total_turns += result.turns as u64;
            min_length = min_length.min(result.turns);
            max_length = max_length.max(result.turns);

            match result.winner {
                Some(winner_idx) if winner_idx < agent_stats.len() => {
                    // Update winner
                    agent_stats[winner_idx].wins += 1;
                    agent_stats[winner_idx].total_games += 1;
                    agent_stats[winner_idx].total_turns += result.turns as u64;

                    // Update losers
                    for (i, stats) in agent_stats.iter_mut().enumerate() {
                        if i != winner_idx && i < result.num_snakes {
                            stats.losses += 1;
                            stats.total_games += 1;
                            stats.total_turns += result.turns as u64;
                        }
                    }
                }
                _ => {
                    // Draw - all participants get a draw
                    total_draws += 1;
                    for (i, stats) in agent_stats.iter_mut().enumerate() {
                        if i < result.num_snakes {
                            stats.draws += 1;
                            stats.total_games += 1;
                            stats.total_turns += result.turns as u64;
                        }
                    }
                }
            }
        }

        let total_games = results.len() as u32;
        let avg_game_length = if total_games > 0 {
            total_turns as f64 / total_games as f64
        } else {
            0.0
        };

        Self {
            agent_stats,
            total_games,
            total_draws,
            avg_game_length,
            min_game_length: if min_length == u32::MAX { 0 } else { min_length },
            max_game_length: max_length,
        }
    }

    /// Print a formatted summary table
    pub fn print_summary(&self) {
        use colored::Colorize;
        use tabled::{Table, Tabled};

        #[derive(Tabled)]
        struct Row {
            #[tabled(rename = "Agent")]
            name: String,
            #[tabled(rename = "Wins")]
            wins: u32,
            #[tabled(rename = "Losses")]
            losses: u32,
            #[tabled(rename = "Draws")]
            draws: u32,
            #[tabled(rename = "Win Rate")]
            win_rate: String,
            #[tabled(rename = "Avg Length")]
            avg_length: String,
        }

        let rows: Vec<Row> = self
            .agent_stats
            .iter()
            .map(|s| Row {
                name: s.name.clone(),
                wins: s.wins,
                losses: s.losses,
                draws: s.draws,
                win_rate: format!("{:.1}%", s.win_rate() * 100.0),
                avg_length: format!("{:.1}", s.avg_game_length()),
            })
            .collect();

        let table = Table::new(rows).to_string();

        println!("\n{}", "=== Tournament Results ===".green().bold());
        println!("{}", table);
        println!();
        println!(
            "Total games: {} | Draws: {} | Avg length: {:.1} turns",
            self.total_games.to_string().cyan(),
            self.total_draws.to_string().yellow(),
            self.avg_game_length
        );
        println!(
            "Game length range: {} - {} turns",
            self.min_game_length.to_string().cyan(),
            self.max_game_length.to_string().cyan()
        );
    }

    /// Export stats to JSON
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Head-to-head comparison between two agents
#[derive(Clone, Debug)]
pub struct HeadToHeadStats {
    pub agent1_name: String,
    pub agent2_name: String,
    pub agent1_wins: u32,
    pub agent2_wins: u32,
    pub draws: u32,
}

impl HeadToHeadStats {
    pub fn from_results(results: &[GameResult], agent1_name: &str, agent2_name: &str) -> Self {
        let mut agent1_wins = 0;
        let mut agent2_wins = 0;
        let mut draws = 0;

        for result in results {
            match result.winner {
                Some(0) => agent1_wins += 1,
                Some(1) => agent2_wins += 1,
                _ => draws += 1,
            }
        }

        Self {
            agent1_name: agent1_name.to_string(),
            agent2_name: agent2_name.to_string(),
            agent1_wins,
            agent2_wins,
            draws,
        }
    }

    pub fn print_summary(&self) {
        use colored::Colorize;

        println!("\n{}", "=== Head-to-Head Results ===".green().bold());
        println!(
            "{}: {} wins ({:.1}%)",
            self.agent1_name.cyan(),
            self.agent1_wins,
            self.agent1_wins as f64 / (self.agent1_wins + self.agent2_wins + self.draws) as f64 * 100.0
        );
        println!(
            "{}: {} wins ({:.1}%)",
            self.agent2_name.cyan(),
            self.agent2_wins,
            self.agent2_wins as f64 / (self.agent1_wins + self.agent2_wins + self.draws) as f64 * 100.0
        );
        println!("Draws: {}", self.draws.to_string().yellow());
    }
}
