use crate::coach::{CoachCtx, CoachState, GAME_SECONDS, Rotation, fatigue_multiplier};
use crate::models::{
    Game, GameResult, GameStatus, League, PlayEvent, Player, PlayerGameStats, Ratings, Team,
    TeamStats, TeamStrategy,
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

/// Energy a fresh player starts a game with.
const FULL_ENERGY: f64 = 100.0;
const FOUL_OUT: u8 = 6;

/// Mutable per-team state while a game is simulated.
struct TeamSim<'a> {
    team: &'a Team,
    players: &'a [&'a Player],
    eff: Vec<Ratings>,
    lineup: Vec<usize>,
    lines: Vec<PlayerGameStats>,
    energy: Vec<f64>,
    fouls: Vec<u8>,
    seconds: Vec<f64>,
    stint_start: Vec<f64>,
    last_exit: Vec<f64>,
    rotation: Rotation,
    coach: CoachState,
    strategy: TeamStrategy,
    score: u16,
}

impl<'a> TeamSim<'a> {
    fn new(team: &'a Team, players: &'a [&'a Player]) -> Self {
        let rotation = Rotation::new(team, players);
        let lineup = rotation.opening_lineup();
        Self {
            team,
            players,
            eff: players
                .iter()
                .map(|player| player.ratings.clone())
                .collect(),
            lineup,
            lines: empty_player_lines(team, players),
            energy: vec![FULL_ENERGY; players.len()],
            fouls: vec![0; players.len()],
            seconds: vec![0.0; players.len()],
            stint_start: vec![0.0; players.len()],
            last_exit: vec![-1e9; players.len()],
            rotation,
            coach: CoachState::default(),
            strategy: team.strategy.clamped(),
            score: 0,
        }
    }

    fn refresh_effective_ratings(&mut self) {
        for (index, player) in self.players.iter().enumerate() {
            self.eff[index] = effective_ratings(&player.ratings, self.energy[index]);
        }
    }

    /// Ask the coach who should play and swap them in.
    fn substitute(
        &mut self,
        elapsed: f64,
        lead: i32,
        half_start: bool,
        plays: &mut Vec<PlayEvent>,
        scores: (u16, u16),
    ) {
        let ctx = CoachCtx {
            lineup: &self.lineup,
            energy: &self.energy,
            fouls: &self.fouls,
            seconds: &self.seconds,
            stint_start: &self.stint_start,
            last_exit: &self.last_exit,
            elapsed,
            lead,
            half_start,
        };
        let next = self.rotation.next_lineup(&ctx, &mut self.coach);
        if next.len() != 5 {
            return;
        }
        let entering: Vec<usize> = next
            .iter()
            .copied()
            .filter(|index| !self.lineup.contains(index))
            .collect();
        let leaving: Vec<usize> = self
            .lineup
            .iter()
            .copied()
            .filter(|index| !next.contains(index))
            .collect();
        for outgoing in &leaving {
            self.last_exit[*outgoing] = elapsed;
        }
        for (slot, incoming) in entering.iter().enumerate() {
            self.stint_start[*incoming] = elapsed;
            if let Some(outgoing) = leaving.get(slot) {
                let (quarter, clock) = game_clock(elapsed);
                plays.push(PlayEvent {
                    quarter,
                    clock,
                    team_id: self.team.id.clone(),
                    description: format!(
                        "Substitution: {} in for {}",
                        self.players[*incoming].name, self.players[*outgoing].name
                    ),
                    away_score: scores.0,
                    home_score: scores.1,
                });
            }
        }
        self.lineup = next;
    }

    /// Drain the floor, refresh the bench.
    fn update_energy(&mut self, iteration_seconds: f64) {
        let minutes = iteration_seconds / 60.0;
        let strategy_load =
            (1.0 + 0.2 * slider(self.strategy.pace)) * (1.0 + 0.2 * slider(self.strategy.pressure));
        for index in 0..self.energy.len() {
            let endurance = self.players[index].ratings.endurance as f64;
            if self.lineup.contains(&index) {
                let drain_per_minute = (6.4 - 0.04 * endurance) * strategy_load;
                self.energy[index] -= drain_per_minute * minutes;
            } else {
                self.energy[index] += (2.4 + 0.015 * endurance) * minutes;
            }
            self.energy[index] = self.energy[index].clamp(0.0, FULL_ENERGY);
        }
    }

    fn quarter_break(&mut self, halftime: bool) {
        let recovery = if halftime { 22.0 } else { 6.0 };
        for energy in &mut self.energy {
            *energy = (*energy + recovery).min(FULL_ENERGY);
        }
    }

    fn credit_floor_time(&mut self, amount: f64) {
        for index in &self.lineup {
            self.seconds[*index] += amount;
        }
    }
}

/// Map a 0-100 slider to -1.0..=1.0 around neutral 50.
fn slider(value: u8) -> f64 {
    (value.min(100) as f64 - 50.0) / 50.0
}

/// Ratings after fatigue. Skills fall by up to 28%; percentage ratings feel
/// half of that so a gassed shooter is worse, not hopeless.
pub fn effective_ratings(base: &Ratings, energy: f64) -> Ratings {
    let penalty = 1.0 - fatigue_multiplier(energy);
    let skill = 1.0 - penalty;
    let pct = 1.0 - penalty * 0.5;
    let scale = |value: u8, factor: f64| (value as f64 * factor).round().clamp(0.0, 99.0) as u8;
    Ratings {
        two_point_pct: scale(base.two_point_pct, pct),
        three_point_pct: scale(base.three_point_pct, pct),
        ft_pct: scale(base.ft_pct, pct),
        inside_scoring: scale(base.inside_scoring, skill),
        three_tendency: base.three_tendency,
        passing: scale(base.passing, skill),
        ball_handling: scale(base.ball_handling, skill),
        perimeter_defense: scale(base.perimeter_defense, skill),
        interior_defense: scale(base.interior_defense, skill),
        steal: scale(base.steal, skill),
        block: scale(base.block, skill),
        offensive_rebounding: scale(base.offensive_rebounding, skill),
        defensive_rebounding: scale(base.defensive_rebounding, skill),
        endurance: base.endurance,
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Ending {
    Scored,
    FreeThrows,
    DefensiveRebound,
    Turnover { steal: bool },
    Other,
}

struct Outcome {
    points: u16,
    ending: Ending,
}

impl PossessionEngine {
    pub fn simulate(&self, input: &GameSimulationInput<'_>) -> GameResult {
        let mut rng = game_rng(input.seed, &input.game.id);
        let mut home = TeamSim::new(input.home_team, &input.home_players);
        let mut away = TeamSim::new(input.away_team, &input.away_players);
        let pace_shift = (slider(home.strategy.pace) + slider(away.strategy.pace)) / 2.0 * 7.0;
        let possessions = (rng.gen_range(96..=106) as f64 + pace_shift)
            .round()
            .max(80.0) as u16;
        let mut plays: Vec<PlayEvent> = Vec::new();
        let seconds_per_iteration = GAME_SECONDS / possessions as f64;
        let mut home_transition = false;
        let mut away_transition;

        for iteration in 0..possessions {
            let home_elapsed = iteration as f64 * seconds_per_iteration;
            let away_elapsed = home_elapsed + seconds_per_iteration / 2.0;
            let quarter_now = (home_elapsed / 720.0) as u32;
            let quarter_before = ((home_elapsed - seconds_per_iteration).max(0.0) / 720.0) as u32;
            if iteration > 0 && quarter_now > quarter_before {
                let halftime = quarter_now == 2;
                home.quarter_break(halftime);
                away.quarter_break(halftime);
            }
            let half_start = iteration == 0
                || (home_elapsed >= 1440.0 && home_elapsed - seconds_per_iteration < 1440.0);
            let lead = home.score as i32 - away.score as i32;
            let scores = (away.score, home.score);
            if iteration > 0 {
                home.substitute(home_elapsed, lead, half_start, &mut plays, scores);
                away.substitute(home_elapsed, -lead, half_start, &mut plays, scores);
            }
            home.refresh_effective_ratings();
            away.refresh_effective_ratings();
            home.credit_floor_time(seconds_per_iteration);
            away.credit_floor_time(seconds_per_iteration);

            let events_before = plays.len();
            let outcome = simulate_possession(
                &mut home,
                &mut away,
                input.config.home_advantage,
                home_transition,
                &mut rng,
                &mut plays,
                home_elapsed,
            );
            apply_possession_plus_minus(&mut home, &mut away, outcome.points, 0);
            home.score += outcome.points;
            stamp_scores(&mut plays[events_before..], away.score, home.score);
            away_transition = rng.gen_bool(transition_chance(outcome.ending, &away, &home));

            let events_before = plays.len();
            let outcome = simulate_possession(
                &mut away,
                &mut home,
                0,
                away_transition,
                &mut rng,
                &mut plays,
                away_elapsed,
            );
            apply_possession_plus_minus(&mut home, &mut away, 0, outcome.points);
            away.score += outcome.points;
            stamp_scores(&mut plays[events_before..], away.score, home.score);
            home_transition = rng.gen_bool(transition_chance(outcome.ending, &home, &away));

            home.update_energy(seconds_per_iteration);
            away.update_energy(seconds_per_iteration);
        }

        finalize_minutes(&mut home.lines, &home.seconds);
        finalize_minutes(&mut away.lines, &away.seconds);

        if home.score == away.score {
            if rng.gen_bool(0.5) {
                let scorer = add_points_to_best(&mut home.lines, 1);
                apply_possession_plus_minus(&mut home, &mut away, 1, 0);
                home.score += 1;
                push_tiebreak_event(
                    &mut plays,
                    &input.home_team.id,
                    &input.home_players,
                    scorer,
                    away.score,
                    home.score,
                );
            } else {
                let scorer = add_points_to_best(&mut away.lines, 1);
                apply_possession_plus_minus(&mut home, &mut away, 0, 1);
                away.score += 1;
                push_tiebreak_event(
                    &mut plays,
                    &input.away_team.id,
                    &input.away_players,
                    scorer,
                    away.score,
                    home.score,
                );
            }
        }

        let (home_score, away_score) = (home.score, away.score);
        let mut player_stats = home.lines;
        player_stats.extend(away.lines);
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

/// Odds the next possession starts as a fast break.
fn transition_chance(
    ending: Ending,
    next_offense: &TeamSim<'_>,
    last_offense: &TeamSim<'_>,
) -> f64 {
    let base = match ending {
        Ending::DefensiveRebound => 0.08,
        Ending::Turnover { steal: true } => 0.32,
        Ending::Turnover { steal: false } => 0.12,
        Ending::Scored | Ending::FreeThrows | Ending::Other => 0.0,
    };
    if base == 0.0 {
        return 0.0;
    }
    // Fast teams run more; a team that crashed the offensive glass has
    // fewer players back; crashing your own defensive glass slows your break.
    (base
        + 0.07 * slider(next_offense.strategy.pace)
        + 0.06 * slider(last_offense.strategy.offensive_glass)
        - 0.05 * slider(next_offense.strategy.defensive_glass))
    .clamp(0.02, 0.5)
}

/// Defensive pressure the shooting side faces, after strategy adjustments.
#[derive(Copy, Clone)]
struct DefensiveContest {
    perimeter: f64,
    interior: f64,
    steal_pressure: f64,
    block_pressure: f64,
}

fn defensive_contest(defense: &TeamSim<'_>) -> DefensiveContest {
    let lineup = &defense.lineup;
    if lineup.is_empty() {
        return DefensiveContest {
            perimeter: 50.0,
            interior: 50.0,
            steal_pressure: 50.0,
            block_pressure: 50.0,
        };
    }
    let count = lineup.len() as f64;
    let average = |pick: fn(&Ratings) -> u8| {
        lineup
            .iter()
            .map(|index| pick(&defense.eff[*index]) as f64)
            .sum::<f64>()
            / count
    };
    let focus = slider(defense.strategy.interior_focus);
    let pressure = slider(defense.strategy.pressure);
    DefensiveContest {
        perimeter: average(|r| r.perimeter_defense) * (1.0 - 0.15 * focus),
        interior: average(|r| r.interior_defense) * (1.0 + 0.15 * focus),
        steal_pressure: average(|r| r.steal) * (1.0 + 0.35 * pressure),
        block_pressure: average(|r| r.block),
    }
}

fn pick_weighted(lineup: &[usize], weights: &[f64], rng: &mut ChaCha8Rng) -> usize {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 || lineup.is_empty() {
        return lineup.first().copied().unwrap_or(0);
    }
    let mut ticket = rng.gen_range(0.0..total);
    for (slot, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            return lineup[slot];
        }
        ticket -= *weight;
    }
    lineup[lineup.len() - 1]
}

fn pick_shooter(offense: &TeamSim<'_>, rng: &mut ChaCha8Rng) -> usize {
    // Low ball movement concentrates shots on the best scorers; high spreads them.
    let exponent = 1.0 - 0.45 * slider(offense.strategy.ball_movement);
    let weights: Vec<f64> = offense
        .lineup
        .iter()
        .map(|index| usage_weight(&offense.eff[*index]).powf(exponent))
        .collect();
    pick_weighted(&offense.lineup, &weights, rng)
}

fn pick_by(team: &TeamSim<'_>, rng: &mut ChaCha8Rng, weight_of: impl Fn(&Ratings) -> f64) -> usize {
    let weights: Vec<f64> = team
        .lineup
        .iter()
        .map(|index| weight_of(&team.eff[*index]) + 5.0)
        .collect();
    pick_weighted(&team.lineup, &weights, rng)
}

/// Charge a personal foul to a defender (never someone already fouled out
/// while an alternative exists).
fn charge_foul(
    defense: &mut TeamSim<'_>,
    rng: &mut ChaCha8Rng,
    plays: &mut Vec<PlayEvent>,
    elapsed: f64,
) -> usize {
    let weights: Vec<f64> = defense
        .lineup
        .iter()
        .map(|index| {
            if defense.fouls[*index] >= FOUL_OUT {
                0.0
            } else {
                let r = &defense.eff[*index];
                45.0 + r.block as f64 * 0.2
                    + r.steal as f64 * 0.15
                    + r.interior_defense as f64 * 0.15
            }
        })
        .collect();
    let fouler = pick_weighted(&defense.lineup, &weights, rng);
    defense.fouls[fouler] += 1;
    defense.lines[fouler].fouls += 1;
    if defense.fouls[fouler] == FOUL_OUT {
        let name = defense.players[fouler].name.clone();
        push_event(
            plays,
            &defense.team.id,
            elapsed,
            format!("{name} fouls out"),
        );
    }
    fouler
}

fn simulate_possession(
    offense: &mut TeamSim<'_>,
    defense: &mut TeamSim<'_>,
    advantage: i16,
    transition: bool,
    rng: &mut ChaCha8Rng,
    plays: &mut Vec<PlayEvent>,
    elapsed: f64,
) -> Outcome {
    let contest = defensive_contest(defense);
    let pressure = slider(defense.strategy.pressure);
    let movement = slider(offense.strategy.ball_movement);
    let offense_team_id = offense.team.id.clone();
    let defense_team_id = defense.team.id.clone();

    // Non-shooting foul: stops play, charges a defender, no free throws.
    if !transition && rng.gen_bool((0.11 + 0.03 * pressure).clamp(0.02, 0.25)) {
        let fouler = charge_foul(defense, rng, plays, elapsed);
        let description = format!(
            "{} personal foul ({})",
            defense.players[fouler].name, defense.fouls[fouler]
        );
        push_event(plays, &defense_team_id, elapsed, description);
    }

    for attempt in 0..3 {
        let fast_break = transition && attempt == 0;
        let shooter_index = pick_shooter(offense, rng);
        let shooter_name = offense.players[shooter_index].name.clone();
        let shooter = offense.eff[shooter_index].clone();

        let foul_chance = (5.0 + shooter.inside_scoring as f64 / 14.0).clamp(5.0, 12.0)
            + 1.5 * pressure
            + if fast_break { 2.0 } else { 0.0 };
        if rng.gen_range(0.0..100.0) < foul_chance {
            let fouler = charge_foul(defense, rng, plays, elapsed);
            let attempts = if rng.gen_bool(0.16) { 3 } else { 2 };
            let made = (0..attempts)
                .filter(|_| rng.gen_range(0..100) < shooter.ft_pct)
                .count() as u16;
            let line = &mut offense.lines[shooter_index];
            line.free_throws_attempted += attempts;
            line.free_throws_made += made;
            line.points += made;
            push_event(
                plays,
                &offense_team_id,
                elapsed,
                format!(
                    "{} makes {} of {} free throws (foul: {}, {})",
                    shooter_name,
                    made,
                    attempts,
                    defense.players[fouler].name,
                    defense.fouls[fouler]
                ),
            );
            return Outcome {
                points: made,
                ending: Ending::FreeThrows,
            };
        }

        let turnover_chance = if fast_break {
            3.0
        } else {
            (11.0 + (contest.steal_pressure - shooter.ball_handling as f64) / 6.0).clamp(7.0, 18.0)
                + 2.2 * pressure
                + 0.8 * movement
        };
        if rng.gen_range(0.0..100.0) < turnover_chance {
            offense.lines[shooter_index].turnovers += 1;
            let mut description = format!("{shooter_name} turnover");
            let mut steal = false;
            if rng.gen_bool((0.70 + 0.10 * pressure).clamp(0.3, 0.9)) {
                let stealer = pick_by(defense, rng, |r| r.steal as f64);
                defense.lines[stealer].steals += 1;
                description = format!(
                    "{} turnover ({} steals)",
                    shooter_name, defense.players[stealer].name
                );
                steal = true;
            }
            push_event(plays, &offense_team_id, elapsed, description);
            return Outcome {
                points: 0,
                ending: Ending::Turnover { steal },
            };
        }

        let three_probability = if fast_break {
            0.0
        } else {
            (0.15 + shooter.three_tendency as f64 / 330.0 - shooter.inside_scoring as f64 / 1400.0
                + contest.perimeter / 3000.0
                + 0.10 * slider(offense.strategy.three_rate))
            .clamp(0.05, 0.6)
        };
        let three = rng.gen_bool(three_probability);
        let mut make_threshold = shot_make_threshold(&shooter, contest, three, advantage);
        make_threshold += 2.0 * movement;
        if fast_break {
            make_threshold += 12.0;
        }
        let make_threshold = make_threshold.clamp(
            if three { 20.0 } else { 30.0 },
            if three { 48.0 } else { 72.0 },
        );
        let shot_label = if three { "three point" } else { "two point" };
        offense.lines[shooter_index].field_goals_attempted += 1;
        if three {
            offense.lines[shooter_index].three_pointers_attempted += 1;
        }

        if rng.gen_range(0.0..100.0) < make_threshold {
            let line = &mut offense.lines[shooter_index];
            line.field_goals_made += 1;
            let points = if three { 3 } else { 2 };
            if three {
                line.three_pointers_made += 1;
            }
            line.points += points;
            let passer = credit_assist(offense, shooter_index, rng);
            let description = match passer {
                Some(passer_index) => format!(
                    "{} makes {} shot ({} assists)",
                    shooter_name, shot_label, offense.players[passer_index].name
                ),
                None => format!("{shooter_name} makes {shot_label} shot"),
            };
            push_event(plays, &offense_team_id, elapsed, description);
            return Outcome {
                points,
                ending: Ending::Scored,
            };
        }

        let block_chance = if three {
            1.0 + contest.block_pressure / 25.0
        } else {
            5.0 + contest.block_pressure / 8.0
        };
        let mut description = format!("{shooter_name} misses {shot_label} shot");
        if rng.gen_range(0.0..100.0) < block_chance {
            let blocker = pick_by(defense, rng, |r| r.block as f64);
            defense.lines[blocker].blocks += 1;
            description = format!(
                "{} blocks {}'s {} shot",
                defense.players[blocker].name, shooter_name, shot_label
            );
        }
        push_event(plays, &offense_team_id, elapsed, description);
        match credit_rebound(offense, defense, rng) {
            Rebound::Offensive(rebounder) => {
                let name = offense.players[rebounder].name.clone();
                push_event(
                    plays,
                    &offense_team_id,
                    elapsed,
                    format!("{name} offensive rebound"),
                );
                continue;
            }
            Rebound::Defensive(rebounder) => {
                let name = defense.players[rebounder].name.clone();
                push_event(
                    plays,
                    &defense_team_id,
                    elapsed,
                    format!("{name} defensive rebound"),
                );
                return Outcome {
                    points: 0,
                    ending: Ending::DefensiveRebound,
                };
            }
            Rebound::None => {}
        }
        return Outcome {
            points: 0,
            ending: Ending::Other,
        };
    }
    Outcome {
        points: 0,
        ending: Ending::Other,
    }
}

fn shot_make_threshold(
    player: &Ratings,
    contest: DefensiveContest,
    three: bool,
    advantage: i16,
) -> f64 {
    let base = if three {
        player.three_point_pct as f64 - 3.0 - (contest.perimeter - 50.0) / 8.0
    } else {
        player.two_point_pct as f64
            - 6.0
            - (contest.interior - 50.0) / 8.0
            - (contest.perimeter - 50.0) / 12.0
            + (player.inside_scoring as f64 - 50.0) / 20.0
    };
    base + advantage as f64 / 2.0
}

fn credit_assist(
    offense: &mut TeamSim<'_>,
    shooter_index: usize,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    let chance = 0.58 + 0.15 * slider(offense.strategy.ball_movement);
    if offense.lineup.len() <= 1 || !rng.gen_bool(chance) {
        return None;
    }
    let mut passer = pick_by(offense, rng, |r| r.passing as f64);
    if passer == shooter_index {
        let slot = offense
            .lineup
            .iter()
            .position(|index| *index == shooter_index)
            .unwrap_or(0);
        passer = offense.lineup[(slot + 1) % offense.lineup.len()];
    }
    offense.lines[passer].assists += 1;
    Some(passer)
}

#[derive(Copy, Clone)]
enum Rebound {
    Offensive(usize),
    Defensive(usize),
    None,
}

fn credit_rebound(
    offense: &mut TeamSim<'_>,
    defense: &mut TeamSim<'_>,
    rng: &mut ChaCha8Rng,
) -> Rebound {
    if offense.lineup.is_empty() || defense.lineup.is_empty() {
        return Rebound::None;
    }
    let offense_strength = offense
        .lineup
        .iter()
        .map(|index| {
            offense.eff[*index].offensive_rebounding as f64
                + offense.eff[*index].inside_scoring as f64 / 4.0
        })
        .sum::<f64>()
        / offense.lineup.len() as f64;
    let defense_strength = defense
        .lineup
        .iter()
        .map(|index| defense.eff[*index].defensive_rebounding as f64)
        .sum::<f64>()
        / defense.lineup.len() as f64;
    let offense_share = (25.0
        + (offense_strength - defense_strength) * 0.18
        + 7.0 * slider(offense.strategy.offensive_glass)
        - 5.0 * slider(defense.strategy.defensive_glass))
    .clamp(12.0, 42.0);
    if rng.gen_bool(offense_share / 100.0) {
        let rebounder = pick_by(offense, rng, |r| r.offensive_rebounding as f64);
        offense.lines[rebounder].rebounds += 1;
        Rebound::Offensive(rebounder)
    } else {
        let rebounder = pick_by(defense, rng, |r| r.defensive_rebounding as f64);
        defense.lines[rebounder].rebounds += 1;
        Rebound::Defensive(rebounder)
    }
}

fn usage_weight(r: &Ratings) -> f64 {
    (r.inside_scoring as f64 * 2.0
        + r.three_tendency as f64
        + r.three_point_pct as f64
        + r.two_point_pct as f64)
        .max(1.0)
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
    home: &mut TeamSim<'_>,
    away: &mut TeamSim<'_>,
    home_points: u16,
    away_points: u16,
) {
    let home_delta = home_points as i16 - away_points as i16;
    let away_delta = -home_delta;
    for index in &home.lineup {
        home.lines[*index].plus_minus += home_delta;
    }
    for index in &away.lineup {
        away.lines[*index].plus_minus += away_delta;
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

fn add_points_to_best(lines: &mut [PlayerGameStats], points: u16) -> Option<usize> {
    let index = (0..lines.len()).max_by_key(|index| lines[*index].minutes)?;
    let line = &mut lines[index];
    line.points += points;
    line.free_throws_attempted += points;
    line.free_throws_made += points;
    Some(index)
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
