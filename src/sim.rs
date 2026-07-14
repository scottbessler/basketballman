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

pub struct GameSimulationInput<'a> {
    pub seed: u64,
    pub game: &'a Game,
    pub home_team: &'a Team,
    pub away_team: &'a Team,
    pub home_players: Vec<&'a Player>,
    pub away_players: Vec<&'a Player>,
    pub config: SimConfig,
}

pub trait GameEngine {
    fn name(&self) -> &'static str;
    fn simulate(&self, input: &GameSimulationInput<'_>) -> GameResult;
}

#[derive(Copy, Clone, Debug)]
pub struct RatingRollEngine;

#[derive(Copy, Clone, Debug)]
pub struct PossessionEngine;

pub fn simulate_game(league: &mut League, game_id: &str, config: SimConfig) -> Option<GameResult> {
    simulate_game_with_engine(league, game_id, &PossessionEngine, config)
}

pub fn simulate_game_with_engine(
    league: &mut League,
    game_id: &str,
    engine: &impl GameEngine,
    config: SimConfig,
) -> Option<GameResult> {
    if let Some(existing) = league.results.get(game_id) {
        return Some(existing.clone());
    }

    let game = league
        .schedule
        .iter()
        .find(|game| game.id == game_id)?
        .clone();
    let result = {
        let input = simulation_input(league, &game, config)?;
        engine.simulate(&input)
    };

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

pub fn simulation_input<'a>(
    league: &'a League,
    game: &'a Game,
    config: SimConfig,
) -> Option<GameSimulationInput<'a>> {
    let home_team = league
        .teams
        .iter()
        .find(|team| team.id == game.home_team_id)?;
    let away_team = league
        .teams
        .iter()
        .find(|team| team.id == game.away_team_id)?;
    Some(GameSimulationInput {
        seed: league.seed,
        game,
        home_team,
        away_team,
        home_players: roster_players(league, home_team),
        away_players: roster_players(league, away_team),
        config,
    })
}

impl GameEngine for RatingRollEngine {
    fn name(&self) -> &'static str {
        "rating-roll"
    }

    fn simulate(&self, input: &GameSimulationInput<'_>) -> GameResult {
        let mut rng = game_rng(input.seed, &input.game.id, self.name());
        let home_rating = team_rating_from_players(&input.home_players);
        let away_rating = team_rating_from_players(&input.away_players);
        let pace = rng.gen_range(94..=104);
        let home_noise = rng.gen_range(-input.config.variance..=input.config.variance);
        let away_noise = rng.gen_range(-input.config.variance..=input.config.variance);
        let mut home_score = 96
            + ((home_rating - away_rating) / 3)
            + input.config.home_advantage
            + home_noise
            + pace / 8;
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
            input.home_team,
            &input.home_players,
            home_score as u16,
            &mut rng,
        ));
        player_stats.extend(simulate_team_player_stats(
            input.away_team,
            &input.away_players,
            away_score as u16,
            &mut rng,
        ));

        result_from_scores(
            input,
            home_score as u16,
            away_score as u16,
            pace as u16,
            player_stats,
        )
    }
}

impl GameEngine for PossessionEngine {
    fn name(&self) -> &'static str {
        "possession"
    }

    fn simulate(&self, input: &GameSimulationInput<'_>) -> GameResult {
        let mut rng = game_rng(input.seed, &input.game.id, self.name());
        let possessions = rng.gen_range(96..=106);
        let mut home_lines = empty_player_lines(input.home_team, &input.home_players);
        let mut away_lines = empty_player_lines(input.away_team, &input.away_players);
        let mut home_seconds = vec![0.0; input.home_players.len()];
        let mut away_seconds = vec![0.0; input.away_players.len()];
        let mut home_lineup = starting_lineup(&input.home_players);
        let mut away_lineup = starting_lineup(&input.away_players);
        let home_targets = target_seconds(&input.home_players);
        let away_targets = target_seconds(&input.away_players);
        let mut home_score = 0u16;
        let mut away_score = 0u16;
        let seconds_per_iteration = 2880.0 / possessions as f64;

        for _ in 0..possessions {
            credit_floor_time(&home_lineup, &mut home_seconds, seconds_per_iteration);
            credit_floor_time(&away_lineup, &mut away_seconds, seconds_per_iteration);
            home_score += simulate_possession(
                &input.home_players,
                &home_lineup,
                &input.away_players,
                &away_lineup,
                &mut home_lines,
                input.config.home_advantage,
                &mut rng,
            );
            away_score += simulate_possession(
                &input.away_players,
                &away_lineup,
                &input.home_players,
                &home_lineup,
                &mut away_lines,
                0,
                &mut rng,
            );
            substitute(
                &mut home_lineup,
                &home_seconds,
                &home_targets,
                seconds_per_iteration,
            );
            substitute(
                &mut away_lineup,
                &away_seconds,
                &away_targets,
                seconds_per_iteration,
            );
        }

        finalize_minutes(&mut home_lines, &home_seconds);
        finalize_minutes(&mut away_lines, &away_seconds);

        if home_score == away_score {
            if rng.gen_bool(0.5) {
                add_points_to_best(&mut home_lines, 1);
                home_score += 1;
            } else {
                add_points_to_best(&mut away_lines, 1);
                away_score += 1;
            }
        }

        let mut player_stats = home_lines;
        player_stats.extend(away_lines);
        result_from_scores(input, home_score, away_score, possessions, player_stats)
    }
}

fn simulate_possession(
    offense: &[&Player],
    offense_lineup: &[usize],
    defense: &[&Player],
    defense_lineup: &[usize],
    lines: &mut [PlayerGameStats],
    advantage: i16,
    rng: &mut ChaCha8Rng,
) -> u16 {
    let shooter_index = weighted_player_index(offense, offense_lineup, rng);
    let shooter = offense[shooter_index];
    let avg_defense = average_defense(defense, defense_lineup);
    let foul_roll = rng.gen_range(0..100);

    if foul_roll < 8 {
        let attempts = if rng.gen_bool(0.22) { 3 } else { 2 };
        let made = (0..attempts)
            .filter(|_| rng.gen_range(0..100) < shooter.ratings.shooting.saturating_add(12))
            .count() as u16;
        lines[shooter_index].free_throws_attempted += attempts;
        lines[shooter_index].free_throws_made += made;
        lines[shooter_index].points += made;
        return made;
    }

    if rng.gen_range(0..100) < turnover_chance(shooter, avg_defense) {
        lines[shooter_index].turnovers += 1;
        return 0;
    }

    let three = rng.gen_bool((0.26 + shooter.ratings.shooting as f64 / 500.0).min(0.46));
    let make_threshold = shot_make_threshold(shooter, avg_defense, three, advantage);
    lines[shooter_index].field_goals_attempted += 1;
    if three {
        lines[shooter_index].three_pointers_attempted += 1;
    }

    if rng.gen_range(0..100) < make_threshold {
        lines[shooter_index].field_goals_made += 1;
        let points = if three { 3 } else { 2 };
        if three {
            lines[shooter_index].three_pointers_made += 1;
        }
        lines[shooter_index].points += points;
        credit_assist(offense, offense_lineup, lines, shooter_index, rng);
        points
    } else {
        credit_rebound(lines, offense, offense_lineup, rng);
        0
    }
}

fn result_from_scores(
    input: &GameSimulationInput<'_>,
    home_score: u16,
    away_score: u16,
    possessions: u16,
    player_stats: Vec<PlayerGameStats>,
) -> GameResult {
    let winner_team_id = if home_score > away_score {
        input.game.home_team_id.clone()
    } else {
        input.game.away_team_id.clone()
    };

    GameResult {
        game_id: input.game.id.clone(),
        home_score,
        away_score,
        winner_team_id,
        team_stats: Some(TeamStats {
            possessions,
            offensive_rating: rating_to_u16(team_rating_from_players(&input.home_players)),
            defensive_rating: rating_to_u16(team_rating_from_players(&input.away_players)),
        }),
        player_stats: Some(player_stats),
    }
}

fn simulate_team_player_stats(
    team: &Team,
    players: &[&Player],
    target_points: u16,
    rng: &mut ChaCha8Rng,
) -> Vec<PlayerGameStats> {
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

fn empty_player_lines(team: &Team, players: &[&Player]) -> Vec<PlayerGameStats> {
    players
        .iter()
        .map(|player| PlayerGameStats {
            player_id: player.id.clone(),
            team_id: team.id.clone(),
            minutes: 0,
            points: 0,
            rebounds: 0,
            assists: 0,
            steals: 0,
            blocks: 0,
            turnovers: 0,
            fouls: 0,
            field_goals_attempted: 0,
            field_goals_made: 0,
            three_pointers_attempted: 0,
            three_pointers_made: 0,
            free_throws_attempted: 0,
            free_throws_made: 0,
        })
        .collect()
}

fn roster_players<'a>(league: &'a League, team: &Team) -> Vec<&'a Player> {
    team.roster
        .iter()
        .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
        .collect()
}

fn weighted_player_index(players: &[&Player], lineup: &[usize], rng: &mut ChaCha8Rng) -> usize {
    let weights: Vec<u16> = lineup
        .iter()
        .map(|index| {
            let player = players[*index];
            player.ratings.offense as u16
                + player.ratings.shooting as u16
                + player.ratings.playmaking as u16 / 2
        })
        .collect();
    let total: u16 = weights.iter().sum();
    let mut ticket = rng.gen_range(0..total.max(1));
    for (slot, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            return lineup[slot];
        }
        ticket -= *weight;
    }
    lineup.last().copied().unwrap_or(0)
}

fn average_defense(players: &[&Player], lineup: &[usize]) -> u8 {
    if lineup.is_empty() {
        return 50;
    }
    (lineup
        .iter()
        .map(|index| players[*index].ratings.defense as u16)
        .sum::<u16>()
        / lineup.len() as u16) as u8
}

fn turnover_chance(player: &Player, avg_defense: u8) -> u8 {
    (14 + avg_defense.saturating_sub(player.ratings.playmaking) / 5).clamp(7, 20)
}

fn shot_make_threshold(player: &Player, avg_defense: u8, three: bool, advantage: i16) -> u8 {
    let base = if three { 33 } else { 47 };
    let rating = if three {
        player.ratings.shooting
    } else {
        ((player.ratings.offense as u16 + player.ratings.shooting as u16) / 2) as u8
    };
    (base + (rating as i16 - avg_defense as i16) / 5 + advantage / 2).clamp(22, 68) as u8
}

fn credit_assist(
    players: &[&Player],
    lineup: &[usize],
    lines: &mut [PlayerGameStats],
    shooter_index: usize,
    rng: &mut ChaCha8Rng,
) {
    if lineup.len() <= 1 || !rng.gen_bool(0.58) {
        return;
    }
    let mut passer_index = weighted_player_index(players, lineup, rng);
    if passer_index == shooter_index {
        let shooter_slot = lineup
            .iter()
            .position(|index| *index == shooter_index)
            .unwrap_or(0);
        passer_index = lineup[(shooter_slot + 1) % lineup.len()];
    }
    lines[passer_index].assists += 1;
}

fn credit_rebound(
    lines: &mut [PlayerGameStats],
    players: &[&Player],
    lineup: &[usize],
    rng: &mut ChaCha8Rng,
) {
    if lineup.is_empty() {
        return;
    }
    let weights: Vec<u16> = lineup
        .iter()
        .map(|index| players[*index].ratings.rebounding as u16 + 10)
        .collect();
    let total: u16 = weights.iter().sum();
    let mut ticket = rng.gen_range(0..total.max(1));
    for (slot, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            lines[lineup[slot]].rebounds += 1;
            return;
        }
        ticket -= *weight;
    }
}

fn add_points_to_best(lines: &mut [PlayerGameStats], points: u16) {
    if let Some(line) = lines.iter_mut().max_by_key(|line| line.minutes) {
        line.points += points;
        line.free_throws_attempted += points;
        line.free_throws_made += points;
    }
}

fn minute_distribution(count: usize) -> Vec<u16> {
    let base = [34, 32, 30, 28, 26, 22, 18, 16, 12, 10, 8, 4];
    (0..count)
        .map(|index| *base.get(index).unwrap_or(&0))
        .collect()
}

fn starting_lineup(players: &[&Player]) -> Vec<usize> {
    let mut lineup: Vec<usize> = (0..players.len()).collect();
    lineup.sort_by_key(|index| {
        (
            std::cmp::Reverse(player_overall(players[*index])),
            players[*index].id.as_str(),
        )
    });
    lineup.truncate(5);
    lineup
}

fn player_overall(player: &Player) -> u16 {
    (player.ratings.offense as u16
        + player.ratings.defense as u16
        + player.ratings.shooting as u16
        + player.ratings.playmaking as u16
        + player.ratings.rebounding as u16)
        / 5
}

fn target_seconds(players: &[&Player]) -> Vec<f64> {
    let mut weights = vec![0.0; players.len()];
    let mut ranked: Vec<usize> = (0..players.len()).collect();
    ranked.sort_by_key(|index| {
        (
            std::cmp::Reverse(player_overall(players[*index])),
            players[*index].id.as_str(),
        )
    });
    for (rank, index) in ranked.into_iter().enumerate() {
        let role_weight = if rank < 5 {
            1.0
        } else if rank < 9 {
            0.65
        } else {
            0.2
        };
        weights[index] = player_overall(players[index]) as f64 * role_weight;
    }
    let total: f64 = weights.iter().sum();
    if total == 0.0 {
        weights.fill(1.0);
    }
    let total = weights.iter().sum::<f64>();
    weights
        .into_iter()
        .map(|weight| weight / total * 5.0 * 2880.0)
        .collect()
}

fn credit_floor_time(lineup: &[usize], seconds: &mut [f64], amount: f64) {
    for index in lineup {
        seconds[*index] += amount;
    }
}

fn substitute(lineup: &mut [usize], seconds: &[f64], targets: &[f64], iteration_seconds: f64) {
    if lineup.is_empty() {
        return;
    }
    let lineup_set = lineup.to_vec();
    let Some((on_slot, &on_index)) = lineup.iter().enumerate().max_by(|(_, left), (_, right)| {
        (seconds[**left] - targets[**left])
            .partial_cmp(&(seconds[**right] - targets[**right]))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return;
    };
    let Some(bench_index) = (0..seconds.len())
        .filter(|index| !lineup_set.contains(index))
        .min_by(|left, right| {
            (seconds[*left] - targets[*left])
                .partial_cmp(&(seconds[*right] - targets[*right]))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    else {
        return;
    };
    if seconds[on_index] - targets[on_index] >= iteration_seconds
        && targets[bench_index] - seconds[bench_index] >= iteration_seconds
    {
        lineup[on_slot] = bench_index;
    }
}

fn finalize_minutes(lines: &mut [PlayerGameStats], seconds: &[f64]) {
    let raw_minutes: Vec<f64> = seconds.iter().map(|value| value / 60.0).collect();
    let mut minutes: Vec<u16> = raw_minutes
        .iter()
        .map(|value| value.floor() as u16)
        .collect();
    let target_total: usize = 5 * 48;
    let current_total: usize = minutes.iter().map(|value| *value as usize).sum();
    let remaining = target_total.saturating_sub(current_total);
    let mut by_fraction: Vec<usize> = (0..lines.len()).collect();
    by_fraction.sort_by(|left, right| {
        raw_minutes[*right]
            .fract()
            .partial_cmp(&raw_minutes[*left].fract())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.cmp(right))
    });
    for index in by_fraction.into_iter().take(remaining) {
        minutes[index] += 1;
    }
    for (line, minute) in lines.iter_mut().zip(minutes) {
        line.minutes = minute.min(48);
    }
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
    team_rating_from_players(&roster_players(league, team))
}

fn team_rating_from_players(players: &[&Player]) -> i16 {
    let mut total = 0i16;
    let mut count = 0i16;
    for player in players {
        total += (player.ratings.offense as i16
            + player.ratings.defense as i16
            + player.ratings.shooting as i16
            + player.ratings.playmaking as i16
            + player.ratings.rebounding as i16)
            / 5;
        count += 1;
    }
    if count == 0 { 50 } else { total / count }
}

fn rating_to_u16(value: i16) -> u16 {
    value.max(0) as u16
}

fn game_rng(league_seed: u64, game_id: &str, engine_name: &str) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(stable_game_seed(league_seed, game_id, engine_name))
}

fn stable_game_seed(league_seed: u64, game_id: &str, engine_name: &str) -> u64 {
    game_id
        .bytes()
        .chain(engine_name.bytes())
        .fold(league_seed ^ 0x9E37_79B9_7F4A_7C15, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte as u64)
        })
}
