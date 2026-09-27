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
            min_game_length: if min_length == u32::MAX {
                0
            } else {
                min_length
            },
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
                // `None` covers both genuine mutual elimination and games that
                // hit the turn cap with more than one survivor. Both are draws
                // for scoring purposes.
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

    /// Total number of games that contributed to this comparison.
    pub fn total_games(&self) -> u32 {
        self.agent1_wins + self.agent2_wins + self.draws
    }

    /// Fraction of games won outright. Draws are in the denominator, so this is
    /// a win rate, not a score.
    pub fn win_rate(&self, wins: u32) -> f64 {
        let total = self.total_games();
        if total == 0 {
            0.0
        } else {
            wins as f64 / total as f64
        }
    }

    /// Game score with a draw worth half a point, the standard expectation for
    /// an even game. This does not reward or punish turn-cap draws as wins.
    pub fn score(&self, wins: u32) -> f64 {
        let total = self.total_games();
        if total == 0 {
            0.0
        } else {
            (wins as f64 + self.draws as f64 / 2.0) / total as f64
        }
    }

    pub fn print_summary(&self) {
        use colored::Colorize;

        println!("\n{}", "=== Head-to-Head Results ===".green().bold());
        println!(
            "{}: {} wins ({:.1}% win, {:.1}% score)",
            self.agent1_name.cyan(),
            self.agent1_wins,
            self.win_rate(self.agent1_wins) * 100.0,
            self.score(self.agent1_wins) * 100.0
        );
        println!(
            "{}: {} wins ({:.1}% win, {:.1}% score)",
            self.agent2_name.cyan(),
            self.agent2_wins,
            self.win_rate(self.agent2_wins) * 100.0,
            self.score(self.agent2_wins) * 100.0
        );
        println!(
            "Draws: {} of {} games",
            self.draws.to_string().yellow(),
            self.total_games()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(winner: Option<usize>, turns: u32, num_snakes: usize) -> GameResult {
        GameResult {
            winner,
            turns,
            num_snakes,
        }
    }

    #[test]
    fn head_to_head_counts_win_loss_and_draw() {
        let results = [
            result(Some(0), 10, 2),
            result(Some(1), 20, 2),
            result(None, 30, 2),
            result(None, 40, 2),
        ];
        let h2h = HeadToHeadStats::from_results(&results, "a", "b");

        assert_eq!(h2h.agent1_wins, 1);
        assert_eq!(h2h.agent2_wins, 1);
        assert_eq!(h2h.draws, 2);
        assert_eq!(h2h.total_games(), 4);
        assert!((h2h.win_rate(1) - 0.25).abs() < 1e-9);
        // One win plus two half-point draws out of four games.
        assert!((h2h.score(1) - 0.5).abs() < 1e-9);
        assert!((h2h.score(0) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn head_to_head_winner_outside_two_seats_is_a_draw_not_a_misattributed_win() {
        // A four-snake result has no meaning in a two-agent comparison.
        let results = [result(Some(2), 15, 4)];
        let h2h = HeadToHeadStats::from_results(&results, "a", "b");
        assert_eq!(h2h.agent1_wins, 0);
        assert_eq!(h2h.agent2_wins, 0);
        assert_eq!(h2h.draws, 1);
    }

    #[test]
    fn head_to_head_empty_results_do_not_divide_by_zero() {
        let h2h = HeadToHeadStats::from_results(&[], "a", "b");
        assert_eq!(h2h.total_games(), 0);
        assert_eq!(h2h.win_rate(0), 0.0);
        assert_eq!(h2h.score(0), 0.0);
    }

    #[test]
    fn tournament_credits_all_survivors_of_a_turn_cap_draw() {
        let results = [result(None, 100, 4), result(Some(0), 50, 4)];
        let names = ["a", "b", "c", "d"].map(String::from);
        let stats = TournamentStats::from_results(&results, &names);

        assert_eq!(stats.total_games, 2);
        assert_eq!(stats.total_draws, 1);
        for (i, agent) in stats.agent_stats.iter().enumerate() {
            assert_eq!(agent.total_games, 2, "agent {i} should play both games");
            assert_eq!(agent.draws, 1);
        }
        assert_eq!(stats.agent_stats[0].wins, 1);
        assert_eq!(stats.agent_stats[1].losses, 1);
        assert_eq!(stats.agent_stats[2].losses, 1);
        assert_eq!(stats.agent_stats[3].losses, 1);
    }
}
