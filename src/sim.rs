use crate::models::{
    Game, GameResult, GameStatus, League, PlayEvent, Player, PlayerGameStats, Team, TeamStats,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Copy, Clone, Debug)]
pub struct SimConfig {
    pub home_advantage: i16,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { home_advantage: 3 }
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

#[derive(Copy, Clone, Debug)]
pub struct PossessionEngine;

pub fn simulate_game(league: &mut League, game_id: &str, config: SimConfig) -> Option<GameResult> {
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
        PossessionEngine.simulate(&input)
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

impl PossessionEngine {
    pub fn simulate(&self, input: &GameSimulationInput<'_>) -> GameResult {
        let mut rng = game_rng(input.seed, &input.game.id);
        let possessions = rng.gen_range(96..=106);
        let mut home_lines = empty_player_lines(input.home_team, &input.home_players);
        let mut away_lines = empty_player_lines(input.away_team, &input.away_players);
        let mut home_seconds = vec![0.0; input.home_players.len()];
        let mut away_seconds = vec![0.0; input.away_players.len()];
        let mut home_lineup = starting_lineup(input.home_team, &input.home_players);
        let mut away_lineup = starting_lineup(input.away_team, &input.away_players);
        let home_targets = target_seconds(input.home_team, &input.home_players);
        let away_targets = target_seconds(input.away_team, &input.away_players);
        let mut home_score = 0u16;
        let mut away_score = 0u16;
        let mut plays: Vec<PlayEvent> = Vec::new();
        let seconds_per_iteration = 2880.0 / possessions as f64;

        for iteration in 0..possessions {
            let home_elapsed = iteration as f64 * seconds_per_iteration;
            let away_elapsed = home_elapsed + seconds_per_iteration / 2.0;
            credit_floor_time(&home_lineup, &mut home_seconds, seconds_per_iteration);
            credit_floor_time(&away_lineup, &mut away_seconds, seconds_per_iteration);
            let events_before = plays.len();
            let home_points = simulate_possession(
                &input.home_players,
                &home_lineup,
                &input.away_players,
                &away_lineup,
                &mut home_lines,
                &mut away_lines,
                input.config.home_advantage,
                &mut rng,
                &mut plays,
                PossessionTeams {
                    offense_team_id: &input.home_team.id,
                    defense_team_id: &input.away_team.id,
                },
                home_elapsed,
            );
            apply_possession_plus_minus(
                &mut home_lines,
                &home_lineup,
                &mut away_lines,
                &away_lineup,
                home_points,
                0,
            );
            home_score += home_points;
            stamp_scores(&mut plays[events_before..], away_score, home_score);
            let events_before = plays.len();
            let away_points = simulate_possession(
                &input.away_players,
                &away_lineup,
                &input.home_players,
                &home_lineup,
                &mut away_lines,
                &mut home_lines,
                0,
                &mut rng,
                &mut plays,
                PossessionTeams {
                    offense_team_id: &input.away_team.id,
                    defense_team_id: &input.home_team.id,
                },
                away_elapsed,
            );
            apply_possession_plus_minus(
                &mut home_lines,
                &home_lineup,
                &mut away_lines,
                &away_lineup,
                0,
                away_points,
            );
            away_score += away_points;
            stamp_scores(&mut plays[events_before..], away_score, home_score);
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
                let scorer = add_points_to_best(&mut home_lines, 1);
                apply_possession_plus_minus(
                    &mut home_lines,
                    &home_lineup,
                    &mut away_lines,
                    &away_lineup,
                    1,
                    0,
                );
                home_score += 1;
                push_tiebreak_event(
                    &mut plays,
                    &input.home_team.id,
                    &input.home_players,
                    scorer,
                    away_score,
                    home_score,
                );
            } else {
                let scorer = add_points_to_best(&mut away_lines, 1);
                apply_possession_plus_minus(
                    &mut home_lines,
                    &home_lineup,
                    &mut away_lines,
                    &away_lineup,
                    0,
                    1,
                );
                away_score += 1;
                push_tiebreak_event(
                    &mut plays,
                    &input.away_team.id,
                    &input.away_players,
                    scorer,
                    away_score,
                    home_score,
                );
            }
        }

        let mut player_stats = home_lines;
        player_stats.extend(away_lines);
        result_from_scores(
            input,
            home_score,
            away_score,
            possessions,
            player_stats,
            plays,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn simulate_possession(
    offense: &[&Player],
    offense_lineup: &[usize],
    defense: &[&Player],
    defense_lineup: &[usize],
    lines: &mut [PlayerGameStats],
    defense_lines: &mut [PlayerGameStats],
    advantage: i16,
    rng: &mut ChaCha8Rng,
    plays: &mut Vec<PlayEvent>,
    teams: PossessionTeams<'_>,
    elapsed_seconds: f64,
) -> u16 {
    for _ in 0..=2 {
        let shooter_index = weighted_player_index(offense, offense_lineup, rng);
        let shooter = offense[shooter_index];
        let contest = average_defensive_contest(defense, defense_lineup);
        let foul_chance = (5 + shooter.ratings.inside_scoring / 14).clamp(5, 12);

        if rng.gen_range(0..100) < foul_chance {
            let attempts = if rng.gen_bool(0.16) { 3 } else { 2 };
            let made = (0..attempts)
                .filter(|_| rng.gen_range(0..100) < shooter.ratings.ft_pct)
                .count() as u16;
            lines[shooter_index].free_throws_attempted += attempts;
            lines[shooter_index].free_throws_made += made;
            lines[shooter_index].points += made;
            push_event(
                plays,
                teams.offense_team_id,
                elapsed_seconds,
                format!(
                    "{} makes {} of {} free throws",
                    shooter.name, made, attempts
                ),
            );
            return made;
        }

        if rng.gen_range(0..100) < turnover_chance(shooter, contest.steal_pressure) {
            lines[shooter_index].turnovers += 1;
            let mut description = format!("{} turnover", shooter.name);
            if rng.gen_bool(0.70)
                && let Some(stealer) = weighted_defender_index(defense, defense_lineup, rng, |p| {
                    p.ratings.steal as u16
                })
            {
                defense_lines[stealer].steals += 1;
                description = format!(
                    "{} turnover ({} steals)",
                    shooter.name, defense[stealer].name
                );
            }
            push_event(plays, teams.offense_team_id, elapsed_seconds, description);
            return 0;
        }

        let three_probability = (0.15 + shooter.ratings.three_tendency as f64 / 330.0
            - shooter.ratings.inside_scoring as f64 / 1400.0
            + contest.perimeter as f64 / 3000.0)
            .clamp(0.15, 0.45);
        let three = rng.gen_bool(three_probability);
        let make_threshold = shot_make_threshold(shooter, contest, three, advantage);
        let shot_label = if three { "three point" } else { "two point" };
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
            let passer = credit_assist(offense, offense_lineup, lines, shooter_index, rng);
            let description = match passer {
                Some(passer_index) => format!(
                    "{} makes {} shot ({} assists)",
                    shooter.name, shot_label, offense[passer_index].name
                ),
                None => format!("{} makes {} shot", shooter.name, shot_label),
            };
            push_event(plays, teams.offense_team_id, elapsed_seconds, description);
            return points;
        }

        let block_chance = if three {
            1 + contest.block_pressure / 25
        } else {
            5 + contest.block_pressure / 8
        };
        let mut description = format!("{} misses {} shot", shooter.name, shot_label);
        if rng.gen_range(0..100) < block_chance
            && let Some(blocker) =
                weighted_defender_index(defense, defense_lineup, rng, |p| p.ratings.block as u16)
        {
            defense_lines[blocker].blocks += 1;
            description = format!(
                "{} blocks {}'s {} shot",
                defense[blocker].name, shooter.name, shot_label
            );
        }
        push_event(plays, teams.offense_team_id, elapsed_seconds, description);
        match credit_rebound(
            lines,
            defense_lines,
            offense,
            offense_lineup,
            defense,
            defense_lineup,
            rng,
        ) {
            Rebound::Offensive(rebounder) => {
                push_event(
                    plays,
                    teams.offense_team_id,
                    elapsed_seconds,
                    format!("{} offensive rebound", offense[rebounder].name),
                );
                continue;
            }
            Rebound::Defensive(rebounder) => {
                push_event(
                    plays,
                    teams.defense_team_id,
                    elapsed_seconds,
                    format!("{} defensive rebound", defense[rebounder].name),
                );
            }
            Rebound::None => {}
        }
        return 0;
    }
    0
}

#[derive(Copy, Clone)]
struct PossessionTeams<'a> {
    offense_team_id: &'a str,
    defense_team_id: &'a str,
}

fn push_event(
    plays: &mut Vec<PlayEvent>,
    team_id: &str,
    elapsed_seconds: f64,
    description: String,
) {
    let (quarter, clock) = game_clock(elapsed_seconds);
    plays.push(PlayEvent {
        quarter,
        clock,
        team_id: team_id.to_string(),
        description,
        away_score: 0,
        home_score: 0,
    });
}

fn stamp_scores(plays: &mut [PlayEvent], away_score: u16, home_score: u16) {
    for play in plays {
        play.away_score = away_score;
        play.home_score = home_score;
    }
}

fn push_tiebreak_event(
    plays: &mut Vec<PlayEvent>,
    team_id: &str,
    players: &[&Player],
    scorer: Option<usize>,
    away_score: u16,
    home_score: u16,
) {
    let Some(scorer) = scorer else {
        return;
    };
    plays.push(PlayEvent {
        quarter: 4,
        clock: "0:00".to_string(),
        team_id: team_id.to_string(),
        description: format!("{} makes 1 of 1 free throws", players[scorer].name),
        away_score,
        home_score,
    });
}

fn game_clock(elapsed_seconds: f64) -> (u8, String) {
    let elapsed = elapsed_seconds.clamp(0.0, 2879.0);
    let quarter = (elapsed / 720.0) as u8 + 1;
    let remaining = (720.0 - elapsed % 720.0).ceil() as u16;
    (quarter, format!("{}:{:02}", remaining / 60, remaining % 60))
}

fn weighted_defender_index(
    defense: &[&Player],
    defense_lineup: &[usize],
    rng: &mut ChaCha8Rng,
    weight_of: impl Fn(&Player) -> u16,
) -> Option<usize> {
    if defense_lineup.is_empty() {
        return None;
    }
    let weights: Vec<u16> = defense_lineup
        .iter()
        .map(|index| weight_of(defense[*index]).saturating_add(5))
        .collect();
    let total: u16 = weights.iter().sum();
    let mut ticket = rng.gen_range(0..total.max(1));
    for (slot, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            return Some(defense_lineup[slot]);
        }
        ticket -= *weight;
    }
    defense_lineup.last().copied()
}

fn result_from_scores(
    input: &GameSimulationInput<'_>,
    home_score: u16,
    away_score: u16,
    possessions: u16,
    player_stats: Vec<PlayerGameStats>,
    play_by_play: Vec<PlayEvent>,
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
        play_by_play: Some(play_by_play),
    }
}

fn apply_possession_plus_minus(
    home_lines: &mut [PlayerGameStats],
    home_lineup: &[usize],
    away_lines: &mut [PlayerGameStats],
    away_lineup: &[usize],
    home_points: u16,
    away_points: u16,
) {
    let home_delta = home_points as i16 - away_points as i16;
    let away_delta = -home_delta;
    for index in home_lineup {
        home_lines[*index].plus_minus += home_delta;
    }
    for index in away_lineup {
        away_lines[*index].plus_minus += away_delta;
    }
}

fn empty_player_lines(team: &Team, players: &[&Player]) -> Vec<PlayerGameStats> {
    players
        .iter()
        .map(|player| PlayerGameStats {
            player_id: player.id.clone(),
            team_id: team.id.clone(),
            plus_minus: 0,
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
            usage_weight(player)
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

#[derive(Copy, Clone)]
struct DefensiveContest {
    perimeter: u8,
    interior: u8,
    steal_pressure: u8,
    block_pressure: u8,
}

fn average_defensive_contest(players: &[&Player], lineup: &[usize]) -> DefensiveContest {
    if lineup.is_empty() {
        return DefensiveContest {
            perimeter: 50,
            interior: 50,
            steal_pressure: 50,
            block_pressure: 50,
        };
    }
    let count = lineup.len() as u16;
    DefensiveContest {
        perimeter: (lineup
            .iter()
            .map(|index| players[*index].ratings.perimeter_defense as u16)
            .sum::<u16>()
            / count) as u8,
        interior: (lineup
            .iter()
            .map(|index| players[*index].ratings.interior_defense as u16)
            .sum::<u16>()
            / count) as u8,
        steal_pressure: (lineup
            .iter()
            .map(|index| players[*index].ratings.steal as u16)
            .sum::<u16>()
            / count) as u8,
        block_pressure: (lineup
            .iter()
            .map(|index| players[*index].ratings.block as u16)
            .sum::<u16>()
            / count) as u8,
    }
}

fn turnover_chance(player: &Player, steal_pressure: u8) -> u8 {
    (11 + steal_pressure.saturating_sub(player.ratings.ball_handling) / 6).clamp(7, 18)
}

fn shot_make_threshold(
    player: &Player,
    contest: DefensiveContest,
    three: bool,
    advantage: i16,
) -> u8 {
    let base = if three {
        player.ratings.three_point_pct as i16 - 3 - (contest.perimeter as i16 - 50) / 8
    } else {
        player.ratings.two_point_pct as i16
            - 6
            - (contest.interior as i16 - 50) / 8
            - (contest.perimeter as i16 - 50) / 12
            + (player.ratings.inside_scoring as i16 - 50) / 20
    };
    (base + advantage / 2).clamp(if three { 25 } else { 38 }, if three { 48 } else { 67 }) as u8
}

fn credit_assist(
    players: &[&Player],
    lineup: &[usize],
    lines: &mut [PlayerGameStats],
    shooter_index: usize,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    if lineup.len() <= 1 || !rng.gen_bool(0.58) {
        return None;
    }
    let mut passer_index = weighted_passing_index(players, lineup, rng);
    if passer_index == shooter_index {
        let shooter_slot = lineup
            .iter()
            .position(|index| *index == shooter_index)
            .unwrap_or(0);
        passer_index = lineup[(shooter_slot + 1) % lineup.len()];
    }
    lines[passer_index].assists += 1;
    Some(passer_index)
}

fn weighted_passing_index(players: &[&Player], lineup: &[usize], rng: &mut ChaCha8Rng) -> usize {
    let weights: Vec<u16> = lineup
        .iter()
        .map(|index| players[*index].ratings.passing as u16 + 5)
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

fn credit_rebound(
    lines: &mut [PlayerGameStats],
    defense_lines: &mut [PlayerGameStats],
    offense: &[&Player],
    offense_lineup: &[usize],
    defense: &[&Player],
    defense_lineup: &[usize],
    rng: &mut ChaCha8Rng,
) -> Rebound {
    if offense_lineup.is_empty() || defense_lineup.is_empty() {
        return Rebound::None;
    }
    let offense_strength: u16 = offense_lineup
        .iter()
        .map(|index| {
            offense[*index].ratings.offensive_rebounding as u16
                + offense[*index].ratings.inside_scoring as u16 / 4
        })
        .sum();
    let defense_strength: u16 = defense_lineup
        .iter()
        .map(|index| defense[*index].ratings.defensive_rebounding as u16)
        .sum();
    let offense_share = (25.0
        + (offense_strength as f64 / offense_lineup.len() as f64
            - defense_strength as f64 / defense_lineup.len() as f64)
            * 0.18)
        .clamp(15.0, 38.0);
    if rng.gen_bool(offense_share / 100.0) {
        match credit_weighted_rebound(lines, offense, offense_lineup, true, rng) {
            Some(rebounder) => Rebound::Offensive(rebounder),
            None => Rebound::None,
        }
    } else {
        match credit_weighted_rebound(defense_lines, defense, defense_lineup, false, rng) {
            Some(rebounder) => Rebound::Defensive(rebounder),
            None => Rebound::None,
        }
    }
}

#[derive(Copy, Clone)]
enum Rebound {
    Offensive(usize),
    Defensive(usize),
    None,
}

fn credit_weighted_rebound(
    lines: &mut [PlayerGameStats],
    players: &[&Player],
    lineup: &[usize],
    offensive: bool,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    let weights: Vec<u16> = lineup
        .iter()
        .map(|index| {
            if offensive {
                players[*index].ratings.offensive_rebounding as u16 + 5
            } else {
                players[*index].ratings.defensive_rebounding as u16 + 5
            }
        })
        .collect();
    let total: u16 = weights.iter().sum();
    let mut ticket = rng.gen_range(0..total.max(1));
    for (slot, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            lines[lineup[slot]].rebounds += 1;
            return Some(lineup[slot]);
        }
        ticket -= *weight;
    }
    None
}

fn add_points_to_best(lines: &mut [PlayerGameStats], points: u16) -> Option<usize> {
    let index = (0..lines.len()).max_by_key(|index| lines[*index].minutes)?;
    let line = &mut lines[index];
    line.points += points;
    line.free_throws_attempted += points;
    line.free_throws_made += points;
    Some(index)
}

fn starting_lineup(team: &Team, players: &[&Player]) -> Vec<usize> {
    if team.starters.len() == 5 {
        let chosen: Vec<usize> = team
            .starters
            .iter()
            .filter_map(|starter_id| players.iter().position(|player| &player.id == starter_id))
            .collect();
        if chosen.len() == 5 {
            return chosen;
        }
    }
    auto_starting_lineup(players)
}

fn auto_starting_lineup(players: &[&Player]) -> Vec<usize> {
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

pub fn player_overall(player: &Player) -> u16 {
    let r = &player.ratings;
    // (weighted rating sum, weight total) per position.
    let (value, weight) = match player.position {
        crate::models::Position::PG => (
            r.passing as u16 * 2
                + r.ball_handling as u16 * 2
                + r.steal as u16
                + r.perimeter_defense as u16
                + r.three_tendency as u16
                + r.three_point_pct as u16
                + r.inside_scoring as u16,
            9,
        ),
        crate::models::Position::SG => (
            r.three_point_pct as u16 * 2
                + r.three_tendency as u16 * 2
                + r.ball_handling as u16
                + r.perimeter_defense as u16
                + r.inside_scoring as u16
                + r.passing as u16,
            8,
        ),
        crate::models::Position::SF => (
            r.two_point_pct as u16
                + r.three_point_pct as u16
                + r.inside_scoring as u16 * 2
                + r.perimeter_defense as u16
                + r.interior_defense as u16
                + r.defensive_rebounding as u16
                + r.passing as u16,
            8,
        ),
        crate::models::Position::PF => (
            r.inside_scoring as u16 * 2
                + r.two_point_pct as u16
                + r.interior_defense as u16 * 2
                + r.offensive_rebounding as u16
                + r.defensive_rebounding as u16
                + r.block as u16
                + r.three_point_pct as u16,
            9,
        ),
        crate::models::Position::C => (
            r.inside_scoring as u16 * 2
                + r.two_point_pct as u16
                + r.interior_defense as u16 * 2
                + r.block as u16 * 2
                + r.offensive_rebounding as u16
                + r.defensive_rebounding as u16,
            9,
        ),
    };
    value / weight
}

fn usage_weight(player: &Player) -> u16 {
    let r = &player.ratings;
    (r.inside_scoring as u16 * 2
        + r.three_tendency as u16
        + r.three_point_pct as u16
        + r.two_point_pct as u16)
        .max(1)
}

fn target_seconds(team: &Team, players: &[&Player]) -> Vec<f64> {
    let mut seconds = auto_target_seconds(players);
    if team.minute_targets.is_empty() {
        return seconds;
    }
    for (index, player) in players.iter().enumerate() {
        if let Some(minutes) = team.minute_targets.get(&player.id) {
            seconds[index] = (*minutes).min(48) as f64 * 60.0;
        }
    }
    let total: f64 = seconds.iter().sum();
    if total <= 0.0 {
        return auto_target_seconds(players);
    }
    // Rescale so the rotation still fills exactly five positions of floor time.
    let scale = 5.0 * 2880.0 / total;
    for value in &mut seconds {
        *value *= scale;
    }
    seconds
}

fn auto_target_seconds(players: &[&Player]) -> Vec<f64> {
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

pub fn team_rating(league: &League, team: &Team) -> i16 {
    team_rating_from_players(&roster_players(league, team))
}

fn team_rating_from_players(players: &[&Player]) -> i16 {
    let mut total = 0i16;
    let mut count = 0i16;
    for player in players {
        total += player_overall(player) as i16;
        count += 1;
    }
    if count == 0 { 50 } else { total / count }
}

fn rating_to_u16(value: i16) -> u16 {
    value.max(0) as u16
}

fn game_rng(league_seed: u64, game_id: &str) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(stable_game_seed(league_seed, game_id))
}

fn stable_game_seed(league_seed: u64, game_id: &str) -> u64 {
    game_id
        .bytes()
        .chain(b"possession".iter().copied())
        .fold(league_seed ^ 0x9E37_79B9_7F4A_7C15, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte as u64)
        })
}
