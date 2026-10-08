//! Coach AI: decides who is on the floor.
//!
//! Modelled on how NBA staffs actually run a game:
//! - starters open each half and rarely all play together; stars are staggered
//! - first stints are ~6-9 minutes, fatigue and minute budgets trigger subs
//! - foul trouble sits players by quarter (2 in Q1, 3 in Q2, 4 in Q3, 5 in Q4)
//!   unless the game is close late, when the closers stay in
//! - blowouts empty the bench once the lead is out of reach
//!
//! The coach is pure: it reads a [`CoachCtx`] snapshot and returns five
//! roster indices. The game sim owns all state.

use crate::models::{
    CHART_BLOCK_MINUTES, CHART_BLOCKS, CoachSettings, LineupMode, Player, Position, Team,
};
use crate::sim::player_overall;

pub const GAME_SECONDS: f64 = 2880.0;
const BLOCK_SECONDS: f64 = CHART_BLOCK_MINUTES as f64 * 60.0;
const FOUL_OUT: u8 = 6;
/// A sub stays in at least this long, and a benched player stays out at
/// least this long, unless fouls or the game script force a change.
const MIN_STINT: f64 = 120.0;
const MIN_REST: f64 = 90.0;

/// Snapshot of one team's game state handed to the coach each possession.
pub struct CoachCtx<'a> {
    pub lineup: &'a [usize],
    pub energy: &'a [f64],
    pub fouls: &'a [u8],
    pub seconds: &'a [f64],
    pub stint_start: &'a [f64],
    /// Game second each player last left the floor (very negative if never).
    pub last_exit: &'a [f64],
    pub elapsed: f64,
    /// Own score minus opponent score.
    pub lead: i32,
    /// True at tip-off and the start of the third quarter.
    pub half_start: bool,
}

/// Mutable coach memory: players sent to the bench to recover stay there
/// until they have rested up.
#[derive(Clone, Debug, Default)]
pub struct CoachState {
    resting: Vec<bool>,
}

pub struct Rotation {
    pub mode: LineupMode,
    pub settings: CoachSettings,
    pub starters: Vec<usize>,
    /// Target floor seconds per roster index.
    pub targets: Vec<f64>,
    pub chart: Vec<Vec<usize>>,
    values: Vec<f64>,
    positions: Vec<Position>,
    /// Roster indices best to worst.
    rank: Vec<usize>,
}

impl Rotation {
    pub fn new(team: &Team, players: &[&Player]) -> Self {
        let values: Vec<f64> = players
            .iter()
            .map(|player| player_overall(player) as f64)
            .collect();
        let positions: Vec<Position> = players.iter().map(|player| player.position).collect();
        let mut rank: Vec<usize> = (0..players.len()).collect();
        rank.sort_by(|a, b| {
            values[*b]
                .partial_cmp(&values[*a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| players[*a].id.cmp(&players[*b].id))
        });
        let settings = team.coach.clamped();
        let custom_starters = resolve_ids(&team.starters, players).filter(|ids| ids.len() == 5);
        let chart = resolve_chart(team, players);

        let mut mode = team.mode();
        if mode == LineupMode::Chart && chart.is_none() {
            mode = if custom_starters.is_some() {
                LineupMode::Minutes
            } else {
                LineupMode::Auto
            };
        }
        if mode == LineupMode::Minutes && custom_starters.is_none() {
            mode = LineupMode::Auto;
        }

        let auto = auto_targets(&values, &rank, &settings);
        let targets = match mode {
            LineupMode::Auto => auto,
            LineupMode::Minutes => minute_targets(team, players, auto),
            LineupMode::Chart => chart_targets(chart.as_deref().unwrap_or(&[]), players.len()),
        };
        let starters = match mode {
            LineupMode::Auto => balanced_top(&rank, &positions, &|_| true),
            LineupMode::Minutes => custom_starters.unwrap_or_default(),
            LineupMode::Chart => chart
                .as_ref()
                .and_then(|chart| chart.first().cloned())
                .unwrap_or_default(),
        };

        Self {
            mode,
            settings,
            starters,
            targets,
            chart: chart.unwrap_or_default(),
            values,
            positions,
            rank,
        }
    }

    /// Roster indices best to worst by overall.
    pub fn ranked(&self) -> &[usize] {
        &self.rank
    }

    /// Roster indices of the five who open the game.
    pub fn opening_lineup(&self) -> Vec<usize> {
        self.starters.clone()
    }

    /// The five who should be on the floor right now.
    pub fn next_lineup(&self, ctx: &CoachCtx<'_>, state: &mut CoachState) -> Vec<usize> {
        let n = self.values.len();
        if state.resting.len() != n {
            state.resting = vec![false; n];
        }
        self.update_resting(ctx, state);

        let clutch = self.settings.closers && is_clutch(ctx.elapsed, ctx.lead);
        let blowout = self.settings.blowout_bench && is_blowout(ctx.elapsed, ctx.lead);
        let limit = foul_limit(&self.settings, ctx.elapsed, clutch);
        let eligible = |i: usize| ctx.fouls[i] < FOUL_OUT;
        let in_trouble = |i: usize| ctx.fouls[i] >= limit;

        let usable = |i: usize| eligible(i) && !in_trouble(i);

        if blowout {
            // Garbage time: everyone outside the top five, best first.
            let mut order: Vec<usize> = Vec::with_capacity(n);
            order.extend(self.rank.iter().skip(5).copied().filter(|i| usable(*i)));
            order.extend(self.rank.iter().take(5).copied().filter(|i| usable(*i)));
            order.truncate(5);
            return fill_to_five(order, &self.rank, ctx.fouls);
        }

        if clutch {
            let mut pool: Vec<usize> = self
                .rank
                .iter()
                .copied()
                .filter(|i| usable(*i) && ctx.energy[*i] >= 25.0)
                .collect();
            if pool.len() < 5 {
                pool = self.rank.iter().copied().filter(|i| usable(*i)).collect();
            }
            let chosen = balanced_top(&pool, &self.positions, &|_| true);
            return fill_to_five(chosen, &self.rank, ctx.fouls);
        }

        let base: Vec<usize> = if ctx.half_start {
            self.starters.clone()
        } else if self.mode == LineupMode::Chart {
            let block = ((ctx.elapsed / BLOCK_SECONDS) as usize).min(CHART_BLOCKS - 1);
            self.chart[block].clone()
        } else {
            Vec::new()
        };

        if !base.is_empty() {
            // Honor the plan, swapping out only players who cannot play.
            let mut lineup: Vec<usize> = Vec::with_capacity(5);
            let mut replaced = 0;
            for index in &base {
                if eligible(*index) && !in_trouble(*index) {
                    lineup.push(*index);
                } else {
                    replaced += 1;
                }
            }
            if replaced > 0 {
                let mut subs: Vec<usize> = (0..n)
                    .filter(|i| eligible(*i) && !in_trouble(*i) && !base.contains(i))
                    .collect();
                subs.sort_by(|a, b| {
                    self.fresh_value(*b, ctx)
                        .partial_cmp(&self.fresh_value(*a, ctx))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                lineup.extend(subs.into_iter().take(replaced));
            }
            return fill_to_five(lineup, &self.rank, ctx.fouls);
        }

        // Dynamic (Auto / Minutes): score everyone, keep the best five.
        let mut scored: Vec<(usize, f64)> = (0..n)
            .filter(|i| eligible(*i))
            .map(|i| (i, self.score(i, ctx, state, in_trouble(i))))
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let ordered: Vec<usize> = scored.iter().map(|(i, _)| *i).collect();
        let mut chosen = balanced_top(&ordered, &self.positions, &|_| true);

        if self.settings.stagger_stars && self.mode == LineupMode::Auto {
            self.stagger(&mut chosen, &ordered, ctx, state, &in_trouble);
        }

        // Minimum stints: fresh subs stay in; just-rested players stay out.
        let rank_in_order = |i: usize| ordered.iter().position(|j| *j == i).unwrap_or(usize::MAX);
        for index in ctx.lineup {
            let fresh = ctx.elapsed - ctx.stint_start[*index] < MIN_STINT;
            if fresh
                && usable(*index)
                && !chosen.contains(index)
                && let Some(drop) = chosen
                    .iter()
                    .copied()
                    .filter(|i| {
                        !(ctx.lineup.contains(i) && ctx.elapsed - ctx.stint_start[*i] < MIN_STINT)
                    })
                    .max_by_key(|i| rank_in_order(*i))
            {
                chosen.retain(|i| *i != drop);
                chosen.push(*index);
            }
        }
        for index in chosen.clone() {
            let just_left = !ctx.lineup.contains(&index)
                && ctx.elapsed - ctx.last_exit[index] < MIN_REST
                && !ctx.half_start;
            if just_left
                && let Some(replacement) = ordered.iter().copied().find(|i| {
                    !chosen.contains(i)
                        && usable(*i)
                        && (ctx.lineup.contains(i) || ctx.elapsed - ctx.last_exit[*i] >= MIN_REST)
                })
            {
                chosen.retain(|i| *i != index);
                chosen.push(replacement);
            }
        }
        fill_to_five(chosen, &self.rank, ctx.fouls)
    }

    fn fresh_value(&self, index: usize, ctx: &CoachCtx<'_>) -> f64 {
        self.values[index] * fatigue_multiplier(ctx.energy[index])
    }

    fn update_resting(&self, ctx: &CoachCtx<'_>, state: &mut CoachState) {
        let floor = rest_floor(&self.settings);
        for (index, energy) in ctx.energy.iter().enumerate() {
            if *energy < floor {
                state.resting[index] = true;
            } else if *energy >= floor + 20.0 {
                state.resting[index] = false;
            }
        }
    }

    fn score(&self, index: usize, ctx: &CoachCtx<'_>, state: &CoachState, trouble: bool) -> f64 {
        let on_floor = ctx.lineup.contains(&index);
        let (quality_weight, minutes_weight) = match self.mode {
            LineupMode::Minutes => (0.12, 7.0),
            _ => (1.0, 1.6),
        };
        let energy = ctx.energy[index];
        let mut score = self.values[index] * fatigue_multiplier(energy) * quality_weight;
        // Minutes owed against a straight-line pace toward the target.
        let pace = self.targets[index] * ctx.elapsed / GAME_SECONDS;
        score += (pace - ctx.seconds[index]) / 60.0 * minutes_weight;
        if self.mode == LineupMode::Minutes {
            score -= (100.0 - energy) * 0.12;
        }
        if on_floor {
            let stint = (ctx.elapsed - ctx.stint_start[index]) / 60.0;
            // Hysteresis keeps rotations from thrashing; very long stints fade.
            score += 4.5 - (stint - 5.0).max(0.0) * 0.9;
            if stint < 2.0 {
                score += 25.0;
            }
        }
        if state.resting[index] {
            score -= 45.0;
        }
        if trouble {
            score -= 70.0;
        }
        score
    }

    /// Never let both of the two best players sit together.
    fn stagger(
        &self,
        chosen: &mut Vec<usize>,
        ordered: &[usize],
        ctx: &CoachCtx<'_>,
        state: &CoachState,
        in_trouble: &impl Fn(usize) -> bool,
    ) {
        let stars: Vec<usize> = self.rank.iter().take(2).copied().collect();
        if stars.iter().any(|star| chosen.contains(star)) {
            return;
        }
        let Some(star) = stars
            .iter()
            .copied()
            .filter(|star| ctx.fouls[*star] < FOUL_OUT && !in_trouble(*star))
            .filter(|star| !state.resting[*star] || ctx.energy[*star] > 40.0)
            .max_by(|a, b| {
                ctx.energy[*a]
                    .partial_cmp(&ctx.energy[*b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        else {
            return;
        };
        // Drop the lowest-scored chosen player.
        if let Some(last) = ordered.iter().rev().find(|i| chosen.contains(i)) {
            let last = *last;
            chosen.retain(|i| *i != last);
            chosen.push(star);
        }
    }
}

/// Skill multiplier at a given energy (shared with the sim's rating scaling).
pub fn fatigue_multiplier(energy: f64) -> f64 {
    0.72 + 0.28 * (energy / 100.0).clamp(0.0, 1.0)
}

/// Energy below which the coach wants a player on the bench.
pub fn rest_floor(settings: &CoachSettings) -> f64 {
    70.0 - settings.fatigue_tolerance as f64 * 0.4
}

/// Fouls at which a player sits. Quarter limits are [2, 3, 4, 5]; the caution
/// slider shifts them by up to one foul either way. Closers play through four fouls.
pub fn foul_limit(settings: &CoachSettings, elapsed: f64, clutch: bool) -> u8 {
    if clutch {
        // Closers play through four fouls but not a fifth.
        return 5;
    }
    let quarter = ((elapsed / 720.0) as i32).clamp(0, 3);
    let shift = ((50.0 - settings.foul_caution as f64) / 50.0).round() as i32;
    (2 + quarter + shift).clamp(1, FOUL_OUT as i32) as u8
}

/// Last six minutes of a game within ten points.
pub fn is_clutch(elapsed: f64, lead: i32) -> bool {
    elapsed >= GAME_SECONDS - 360.0 && lead.abs() <= 10
}

/// Lead that cannot realistically be erased in the time left.
pub fn is_blowout(elapsed: f64, lead: i32) -> bool {
    if elapsed < 1800.0 {
        return false;
    }
    let minutes_left = (GAME_SECONDS - elapsed) / 60.0;
    lead.abs() as f64 >= 10.0 + 1.25 * minutes_left
}

fn resolve_ids(ids: &[String], players: &[&Player]) -> Option<Vec<usize>> {
    ids.iter()
        .map(|id| players.iter().position(|player| &player.id == id))
        .collect()
}

fn resolve_chart(team: &Team, players: &[&Player]) -> Option<Vec<Vec<usize>>> {
    if team.chart.len() != CHART_BLOCKS {
        return None;
    }
    let mut chart = Vec::with_capacity(CHART_BLOCKS);
    for block in &team.chart {
        let indices = resolve_ids(block, players)?;
        let mut unique = indices.clone();
        unique.sort_unstable();
        unique.dedup();
        if indices.len() != 5 || unique.len() != 5 {
            return None;
        }
        chart.push(indices);
    }
    Some(chart)
}

fn chart_targets(chart: &[Vec<usize>], roster: usize) -> Vec<f64> {
    let mut seconds = vec![0.0; roster];
    for block in chart {
        for index in block {
            seconds[*index] += BLOCK_SECONDS;
        }
    }
    seconds
}

fn minute_targets(team: &Team, players: &[&Player], auto: Vec<f64>) -> Vec<f64> {
    let mut seconds = auto;
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
        return vec![5.0 * GAME_SECONDS / players.len().max(1) as f64; players.len()];
    }
    // Rescale so the rotation still fills exactly five positions of floor time.
    let scale = 5.0 * GAME_SECONDS / total;
    seconds.iter().map(|value| value * scale).collect()
}

/// NBA-shaped minute budget: starters 28-38 min, a tapering bench, deep
/// reserves only in garbage time.
pub fn auto_targets(values: &[f64], rank: &[usize], settings: &CoachSettings) -> Vec<f64> {
    let n = values.len();
    let load = settings.starter_load as f64 / 100.0;
    let depth = (settings.depth as usize).clamp(7, 12).min(n);
    let starter_minutes: [f64; 5] = [
        28.0 + 10.0 * load,
        27.5 + 9.5 * load,
        27.0 + 9.0 * load,
        26.0 + 8.5 * load,
        25.0 + 8.0 * load,
    ];
    // Best player plays the most; scale the five slightly by relative quality
    // so a 90 plays more than a 70 at the same slider setting.
    let mut minutes = vec![0.0; n];
    let mut used = 0.0;
    for (slot, index) in rank.iter().take(5).enumerate() {
        minutes[*index] = starter_minutes[slot];
        used += starter_minutes[slot];
    }
    let bench_slots: Vec<usize> = rank
        .iter()
        .skip(5)
        .take(depth.saturating_sub(5))
        .copied()
        .collect();
    let weights: Vec<f64> = (0..bench_slots.len())
        .map(|slot| 1.0 / (1.0 + slot as f64 * 0.55))
        .collect();
    let weight_total: f64 = weights.iter().sum();
    let remaining = (240.0 - used).max(0.0);
    for (slot, index) in bench_slots.iter().enumerate() {
        minutes[*index] = remaining * weights[slot] / weight_total.max(1e-9);
    }
    // Deep reserves get a token garbage-time allotment.
    let reserves: Vec<usize> = rank.iter().skip(depth).copied().collect();
    for index in &reserves {
        minutes[*index] = 2.0;
    }
    let total: f64 = minutes.iter().sum();
    if total <= 0.0 {
        return vec![5.0 * GAME_SECONDS / n.max(1) as f64; n];
    }
    minutes
        .iter()
        .map(|value| value / total * 5.0 * GAME_SECONDS)
        .collect()
}

/// Best five from `ordered` (best first), keeping at least one ball handler
/// (PG/SG) and one big (PF/C) when the pool allows.
fn balanced_top(
    ordered: &[usize],
    positions: &[Position],
    allowed: &dyn Fn(usize) -> bool,
) -> Vec<usize> {
    let pool: Vec<usize> = ordered.iter().copied().filter(|i| allowed(*i)).collect();
    let mut chosen: Vec<usize> = pool.iter().copied().take(5).collect();
    let is_guard = |i: &usize| matches!(positions[*i], Position::PG | Position::SG);
    let is_big = |i: &usize| matches!(positions[*i], Position::PF | Position::C);
    for want_guard in [true, false] {
        let satisfied = |chosen: &[usize]| {
            if want_guard {
                chosen.iter().any(is_guard)
            } else {
                chosen.iter().any(is_big)
            }
        };
        if chosen.len() < 5 || satisfied(&chosen) {
            continue;
        }
        let Some(candidate) = pool
            .iter()
            .skip(5)
            .find(|i| if want_guard { is_guard(i) } else { is_big(i) })
            .copied()
        else {
            continue;
        };
        // Remove the worst chosen player who is not the only holder of the
        // other requirement.
        let other_is_guard = !want_guard;
        let removable = chosen.iter().rev().copied().find(|i| {
            let other_count = chosen
                .iter()
                .filter(|j| {
                    if other_is_guard {
                        is_guard(j)
                    } else {
                        is_big(j)
                    }
                })
                .count();
            let counts_for_other = if other_is_guard {
                is_guard(i)
            } else {
                is_big(i)
            };
            !(counts_for_other && other_count <= 1)
        });
        if let Some(removed) = removable {
            chosen.retain(|i| *i != removed);
            chosen.push(candidate);
        }
    }
    chosen
}

/// Guarantee exactly five distinct indices, borrowing the best remaining
/// players (fouled-out ones last resort) when the plan came up short.
fn fill_to_five(mut lineup: Vec<usize>, rank: &[usize], fouls: &[u8]) -> Vec<usize> {
    lineup.dedup();
    lineup.truncate(5);
    for pass in 0..2 {
        for index in rank {
            if lineup.len() >= 5 {
                return lineup;
            }
            let ok = pass == 1 || fouls[*index] < FOUL_OUT;
            if ok && !lineup.contains(index) {
                lineup.push(*index);
            }
        }
    }
    lineup
}

/// Build a sensible 12-block rotation chart from the team's coach settings
/// (Auto targets, or its minute targets when it has them): starters open each
/// half, stints run 2-3 blocks, stars are staggered and the best five close
/// each half.
pub fn suggest_chart(team: &Team, players: &[&Player]) -> Vec<Vec<usize>> {
    let mut plan_team = team.clone();
    plan_team.lineup_mode = Some(if team.minute_targets.is_empty() {
        LineupMode::Auto
    } else {
        LineupMode::Minutes
    });
    let rotation = Rotation::new(&plan_team, players);
    let n = players.len();
    let mut need: Vec<f64> = rotation
        .targets
        .iter()
        .map(|seconds| seconds / BLOCK_SECONDS)
        .collect();
    let mut consecutive = vec![0u32; n];
    let mut previous: Vec<usize> = Vec::new();
    let top_two: Vec<usize> = rotation.rank.iter().take(2).copied().collect();
    let mut chart: Vec<Vec<usize>> = Vec::with_capacity(CHART_BLOCKS);

    for block in 0..CHART_BLOCKS {
        let opening = block == 0 || block == CHART_BLOCKS / 2;
        let closing = block == CHART_BLOCKS / 2 - 1 || block == CHART_BLOCKS - 1;
        let mut scored: Vec<(usize, f64)> = (0..n)
            .map(|i| {
                let mut score = need[i] * 3.0 + rotation.values[i] * 0.02;
                if opening && rotation.starters.contains(&i) {
                    score += 100.0;
                }
                if closing && rotation.rank.iter().take(5).any(|top| *top == i) {
                    score += 6.0;
                }
                if previous.contains(&i) {
                    score += 1.5;
                    if consecutive[i] >= 2 {
                        score -= 4.0 + 3.0 * (consecutive[i] - 2) as f64;
                    }
                }
                // Stagger: one of the two best players should always play.
                if top_two.contains(&i) && !opening {
                    let partner_on = top_two.iter().any(|star| {
                        *star != i && previous.contains(star) && consecutive[*star] < 2
                    });
                    if partner_on && previous.contains(&i) && consecutive[i] >= 1 {
                        score -= 2.5;
                    }
                }
                (i, score)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let ordered: Vec<usize> = scored.into_iter().map(|(i, _)| i).collect();
        let mut lineup = balanced_top(&ordered, &rotation.positions, &|_| true);
        if !lineup.iter().any(|i| top_two.contains(i))
            && let Some(star) = top_two.iter().copied().max_by(|a, b| {
                need[*a]
                    .partial_cmp(&need[*b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            && let Some(drop) = ordered.iter().rev().find(|i| lineup.contains(i)).copied()
        {
            lineup.retain(|i| *i != drop);
            lineup.push(star);
        }
        for i in 0..n {
            if lineup.contains(&i) {
                consecutive[i] += 1;
                need[i] -= 1.0;
            } else {
                consecutive[i] = 0;
            }
        }
        previous = lineup.clone();
        chart.push(lineup);
    }
    chart
}
