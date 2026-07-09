use crate::models::{
    Game, GameResult, GameStatus, League, Player, PlayerGameStats, Team, TeamStats,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Copy, Clone, Debug)]
pub struct SimConfig {
    pub home_advantage: i16,
    pub variance: i16,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            home_advantage: 3,
            variance: 11,
        }
    }
}

pub fn simulate_game(league: &mut League, game_id: &str, config: SimConfig) -> Option<GameResult> {
    if let Some(existing) = league.results.get(game_id) {
        return Some(existing.clone());
    }

    let game = league
        .schedule
        .iter()
        .find(|game| game.id == game_id)?
        .clone();
    let home_team = league
        .teams
        .iter()
        .find(|team| team.id == game.home_team_id)?;
    let away_team = league
        .teams
        .iter()
        .find(|team| team.id == game.away_team_id)?;
    let result = simulate_matchup(league, &game, home_team, away_team, config);

    if let Some(stored_game) = league
        .schedule
        .iter_mut()
        .find(|stored| stored.id == game_id)
    {
        stored_game.status = GameStatus::Played;
    }
    league.results.insert(game_id.to_string(), result.clone());

    Some(result)
}

fn simulate_matchup(
    league: &League,
    game: &Game,
    home_team: &Team,
    away_team: &Team,
    config: SimConfig,
) -> GameResult {
    let mut rng = ChaCha8Rng::seed_from_u64(stable_game_seed(league.seed, &game.id));
    let home_rating = team_rating(league, home_team);
    let away_rating = team_rating(league, away_team);
    let pace = rng.gen_range(94..=104);
    let home_noise = rng.gen_range(-config.variance..=config.variance);
    let away_noise = rng.gen_range(-config.variance..=config.variance);
    let mut home_score =
        96 + ((home_rating - away_rating) / 3) + config.home_advantage + home_noise + pace / 8;
    let mut away_score = 96 + ((away_rating - home_rating) / 3) + away_noise + pace / 8;

    home_score = home_score.max(70);
    away_score = away_score.max(70);
    if home_score == away_score {
        if rng.gen_bool(0.5) {
            home_score += 1;
        } else {
            away_score += 1;
        }
    }

    let mut player_stats = Vec::new();
    player_stats.extend(simulate_team_player_stats(
        league,
        home_team,
        home_score as u16,
        &mut rng,
    ));
    player_stats.extend(simulate_team_player_stats(
        league,
        away_team,
        away_score as u16,
        &mut rng,
    ));

    let winner_team_id = if home_score > away_score {
        game.home_team_id.clone()
    } else {
        game.away_team_id.clone()
    };

    GameResult {
        game_id: game.id.clone(),
        home_score: home_score as u16,
        away_score: away_score as u16,
        winner_team_id,
        team_stats: Some(TeamStats {
            possessions: pace as u16,
            offensive_rating: home_rating.max(0) as u16,
            defensive_rating: away_rating.max(0) as u16,
        }),
        player_stats: Some(player_stats),
    }
}

fn simulate_team_player_stats(
    league: &League,
    team: &Team,
    target_points: u16,
    rng: &mut ChaCha8Rng,
) -> Vec<PlayerGameStats> {
    let players: Vec<&Player> = team
        .roster
        .iter()
        .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
        .collect();
    let minutes = minute_distribution(players.len());
    let weights: Vec<u16> = players
        .iter()
        .map(|player| {
            player.ratings.offense as u16
                + player.ratings.shooting as u16
                + player.ratings.playmaking as u16 / 2
                + rng.gen_range(0..=16)
        })
        .collect();
    let mut points = distribute_points(target_points, &weights);

    let point_sum: i16 = points.iter().map(|value| *value as i16).sum();
    let diff = target_points as i16 - point_sum;
    if let Some(top_index) = points
        .iter()
        .enumerate()
        .max_by_key(|(index, value)| (**value, weights[*index]))
        .map(|(index, _)| index)
    {
        points[top_index] = (points[top_index] as i16 + diff).max(0) as u16;
    }

    players
        .iter()
        .enumerate()
        .map(|(index, player)| {
            let pts = points[index];
            let fgm = (pts / 2).max(if pts > 0 { 1 } else { 0 });
            let fga = fgm + rng.gen_range(1..=7);
            let tpm = (pts / 9).min(fgm);
            let tpa = tpm + rng.gen_range(0..=4);
            let ftm = pts.saturating_sub((fgm - tpm) * 2 + tpm * 3);
            let fta = ftm + rng.gen_range(0..=3);
            PlayerGameStats {
                player_id: player.id.clone(),
                team_id: team.id.clone(),
                minutes: minutes[index],
                points: pts,
                rebounds: stat_from_rating(player.ratings.rebounding, minutes[index], 2, 12, rng),
                assists: stat_from_rating(player.ratings.playmaking, minutes[index], 1, 10, rng),
                steals: stat_from_rating(player.ratings.defense, minutes[index], 0, 3, rng),
                blocks: stat_from_rating(player.ratings.defense, minutes[index], 0, 3, rng),
                turnovers: rng.gen_range(0..=4),
                fouls: rng.gen_range(0..=5),
                field_goals_attempted: fga,
                field_goals_made: fgm.min(fga),
                three_pointers_attempted: tpa,
                three_pointers_made: tpm.min(tpa),
                free_throws_attempted: fta,
                free_throws_made: ftm.min(fta),
            }
        })
        .collect()
}

fn minute_distribution(count: usize) -> Vec<u16> {
    let base = [34, 32, 30, 28, 26, 22, 18, 16, 12, 10, 8, 4];
    (0..count)
        .map(|index| *base.get(index).unwrap_or(&0))
        .collect()
}

fn distribute_points(target_points: u16, weights: &[u16]) -> Vec<u16> {
    let total_weight: u16 = weights.iter().sum();
    if total_weight == 0 {
        return vec![0; weights.len()];
    }
    let mut remaining = target_points;
    let mut points = Vec::with_capacity(weights.len());
    for (index, weight) in weights.iter().enumerate() {
        if index == weights.len() - 1 {
            points.push(remaining);
        } else {
            let value = ((target_points as u32 * *weight as u32) / total_weight as u32) as u16;
            let value = value.min(remaining);
            points.push(value);
            remaining -= value;
        }
    }
    points
}

fn stat_from_rating(
    rating: u8,
    minutes: u16,
    floor: u16,
    ceiling: u16,
    rng: &mut ChaCha8Rng,
) -> u16 {
    let scaled = (rating as u16 * minutes) / 220;
    (floor + scaled + rng.gen_range(0..=2)).min(ceiling)
}

pub fn team_rating(league: &League, team: &Team) -> i16 {
    let mut total = 0i16;
    let mut count = 0i16;
    for player_id in &team.roster {
        if let Some(player) = league.players.iter().find(|player| &player.id == player_id) {
            total += (player.ratings.offense as i16
                + player.ratings.defense as i16
                + player.ratings.shooting as i16
                + player.ratings.playmaking as i16
                + player.ratings.rebounding as i16)
                / 5;
            count += 1;
        }
    }
    if count == 0 { 50 } else { total / count }
}

fn stable_game_seed(league_seed: u64, game_id: &str) -> u64 {
    game_id
        .bytes()
        .fold(league_seed ^ 0x9E37_79B9_7F4A_7C15, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte as u64)
        })
}
