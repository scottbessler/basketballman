//! Owner-facing lineup editor: rotation mode, depth order, minute targets,
//! the 12-block rotation chart, team strategy and coach sliders.

use crate::coach::{Rotation, suggest_chart};
use crate::models::{
    CHART_BLOCK_MINUTES, CHART_BLOCKS, CoachSettings, LineupMode, Player, PlayerId, Team,
    TeamStrategy,
};
use crate::routes::{AppState, persist_and_redirect, position_name, render};
use crate::session::{AuthUser, MaybeUser};
use crate::sim::player_overall;
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

const MIN_DEPTH: usize = 5;

/// Apply the editor form to a team. Pure so it can be tested without HTTP.
///
/// Recognised fields: `mode`, repeated `order` (depth, best first),
/// `min_<player>`, repeated `chart_<block>` (player ids), strategy and coach
/// sliders, `coach_form` (marks the toggle checkboxes as present), and the
/// legacy repeated `starter` field. `move=up:<id>` / `move=down:<id>` nudges a
/// player in the depth order without validating the rest of the form.
pub fn apply_lineup_form(
    team: &mut Team,
    roster: &[PlayerId],
    fields: &[(String, String)],
) -> Result<LineupOutcome, String> {
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if field("reset").is_some() {
        team.reset_lineup();
        return Ok(LineupOutcome::Saved);
    }

    // Depth order: posted `order`, else legacy starters, else saved order.
    let mut order: Vec<PlayerId> = Vec::new();
    let push_unique = |order: &mut Vec<PlayerId>, id: &str| {
        if roster.iter().any(|candidate| candidate == id) && !order.iter().any(|o| o == id) {
            order.push(id.to_string());
        }
    };
    let has_order = fields.iter().any(|(key, _)| key == "order");
    if has_order {
        for (key, value) in fields {
            if key == "order" {
                push_unique(&mut order, value);
            }
        }
    } else {
        for (key, value) in fields {
            if key == "starter" {
                push_unique(&mut order, value);
            }
        }
        if order.is_empty() {
            for id in team.starters.iter().chain(&team.bench_order) {
                push_unique(&mut order, id);
            }
        }
    }
    for id in roster {
        push_unique(&mut order, id);
    }

    let mut moved = false;
    if let Some(command) = field("move")
        && let Some((direction, id)) = command.split_once(':')
        && let Some(position) = order.iter().position(|o| o == id)
    {
        let target = match direction {
            "up" => position.checked_sub(1),
            "down" => (position + 1 < order.len()).then_some(position + 1),
            _ => None,
        };
        if let Some(target) = target {
            order.swap(position, target);
            moved = true;
        }
    }

    let mut minute_targets = std::collections::BTreeMap::new();
    for (key, value) in fields {
        if let Some(player_id) = key.strip_prefix("min_")
            && roster.iter().any(|id| id == player_id)
            && let Ok(minutes) = value.trim().parse::<u16>()
        {
            minute_targets.insert(player_id.to_string(), minutes.min(48));
        }
    }

    let mut chart: Vec<Vec<PlayerId>> = vec![Vec::new(); CHART_BLOCKS];
    let mut chart_posted = false;
    for (key, value) in fields {
        if let Some(block) = key.strip_prefix("chart_")
            && let Ok(block) = block.parse::<usize>()
            && block < CHART_BLOCKS
        {
            chart_posted = true;
            if roster.iter().any(|id| id == value) && !chart[block].contains(value) {
                chart[block].push(value.clone());
            }
        }
    }
    let chart_valid = chart.iter().all(|block| block.len() == 5);

    let mode = match field("mode") {
        Some("auto") => LineupMode::Auto,
        Some("minutes") => LineupMode::Minutes,
        Some("chart") => LineupMode::Chart,
        _ if !has_order && fields.iter().any(|(key, _)| key == "starter") => LineupMode::Minutes,
        _ => team.mode(),
    };
    if moved {
        team.starters = order.iter().take(5).cloned().collect();
        team.bench_order = order.iter().skip(5).cloned().collect();
        return Ok(LineupOutcome::Moved);
    }
    if mode == LineupMode::Chart && chart_posted && !chart_valid {
        return Err("Every 4-minute block needs exactly five players on the floor.".to_string());
    }
    if mode == LineupMode::Chart && !chart_posted && team.chart.len() != CHART_BLOCKS {
        return Err("Fill in the rotation chart before using Chart mode.".to_string());
    }
    if order.len() < MIN_DEPTH {
        return Err("A team needs at least five players.".to_string());
    }

    team.starters = order.iter().take(5).cloned().collect();
    team.bench_order = order.iter().skip(5).cloned().collect();
    if !minute_targets.is_empty() {
        team.minute_targets = minute_targets;
    }
    if chart_posted && chart_valid {
        team.chart = chart;
    }
    team.lineup_mode = Some(mode);

    let number = |name: &str, current: u8| {
        field(name)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .map(|value| value.clamp(0, 255) as u8)
            .unwrap_or(current)
    };
    let strategy = team.strategy;
    team.strategy = TeamStrategy {
        pace: number("pace", strategy.pace),
        three_rate: number("three_rate", strategy.three_rate),
        ball_movement: number("ball_movement", strategy.ball_movement),
        offensive_glass: number("offensive_glass", strategy.offensive_glass),
        pressure: number("pressure", strategy.pressure),
        interior_focus: number("interior_focus", strategy.interior_focus),
        defensive_glass: number("defensive_glass", strategy.defensive_glass),
    }
    .clamped();
    let coach = team.coach;
    let toggles_present = field("coach_form").is_some();
    let toggle = |name: &str, current: bool| {
        if toggles_present {
            field(name).is_some()
        } else {
            current
        }
    };
    team.coach = CoachSettings {
        foul_caution: number("foul_caution", coach.foul_caution),
        starter_load: number("starter_load", coach.starter_load),
        depth: number("depth", coach.depth),
        fatigue_tolerance: number("fatigue_tolerance", coach.fatigue_tolerance),
        closers: toggle("closers", coach.closers),
        blowout_bench: toggle("blowout_bench", coach.blowout_bench),
        stagger_stars: toggle("stagger_stars", coach.stagger_stars),
    }
    .clamped();
    Ok(LineupOutcome::Saved)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LineupOutcome {
    Saved,
    Moved,
}

#[derive(Deserialize)]
pub struct EditorQuery {
    #[serde(default)]
    error: String,
    #[serde(default)]
    saved: String,
}

pub async fn editor(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Path(id): Path<String>,
    Query(query): Query<EditorQuery>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    let Some(team) = league.teams.iter().find(|team| team.id == id) else {
        return (StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if viewer.is_none() || team.owner_user_id != viewer {
        return Redirect::to(&format!("/teams/{id}")).into_response();
    }
    let players = roster_players(&league, team);
    render(LineupTemplate::build(
        team,
        &players,
        query.error,
        !query.saved.is_empty(),
    ))
}

pub async fn save(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(index) = league.teams.iter().position(|team| team.id == id) else {
        return (StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if league.teams[index].owner_user_id != Some(user_id) {
        return (StatusCode::FORBIDDEN, "you do not manage this team").into_response();
    }
    let roster = league.teams[index].roster.clone();
    let mut team = league.teams[index].clone();
    match apply_lineup_form(&mut team, &roster, &fields) {
        Ok(outcome) => {
            league.teams[index] = team;
            let target = match outcome {
                LineupOutcome::Saved => format!("/teams/{id}/lineup?saved=1"),
                LineupOutcome::Moved => format!("/teams/{id}/lineup"),
            };
            persist_and_redirect(&state, &league, &target)
        }
        Err(message) => {
            let encoded: String = message
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => (byte as char).to_string(),
                    _ => format!("%{byte:02X}"),
                })
                .collect();
            Redirect::to(&format!("/teams/{id}/lineup?error={encoded}")).into_response()
        }
    }
}

/// Replace the saved rotation chart with the coach's suggestion.
pub async fn suggest(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(index) = league.teams.iter().position(|team| team.id == id) else {
        return (StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if league.teams[index].owner_user_id != Some(user_id) {
        return (StatusCode::FORBIDDEN, "you do not manage this team").into_response();
    }
    let chart = {
        let team = &league.teams[index];
        let players = roster_players(&league, team);
        suggest_chart(team, &players)
            .into_iter()
            .map(|block| block.into_iter().map(|i| players[i].id.clone()).collect())
            .collect::<Vec<Vec<PlayerId>>>()
    };
    league.teams[index].chart = chart;
    persist_and_redirect(
        &state,
        &league,
        &format!("/teams/{id}/lineup?saved=1#chart"),
    )
}

fn roster_players<'a>(league: &'a crate::models::League, team: &Team) -> Vec<&'a Player> {
    team.roster
        .iter()
        .filter_map(|id| league.players.iter().find(|player| &player.id == id))
        .collect()
}

#[derive(Template)]
#[template(path = "lineup.html")]
pub struct LineupTemplate {
    team_id: String,
    team_name: String,
    error: String,
    saved: bool,
    mode: String,
    rows: Vec<DepthRow>,
    chart_rows: Vec<ChartRow>,
    quarters: Vec<QuarterHead>,
    block_numbers: Vec<usize>,
    offense: Vec<SliderView>,
    defense: Vec<SliderView>,
    coach: Vec<SliderView>,
    toggles: Vec<ToggleView>,
    chart_is_saved: bool,
}

struct DepthRow {
    id: String,
    name: String,
    position: String,
    age: u8,
    overall: u16,
    endurance: u8,
    minutes: String,
    starter: bool,
}

struct ChartRow {
    id: String,
    name: String,
    short_name: String,
    position: String,
    cells: Vec<ChartCell>,
    minutes: u16,
}

struct ChartCell {
    block: usize,
    on: bool,
}

struct QuarterHead {
    label: String,
}

struct SliderView {
    name: &'static str,
    label: &'static str,
    low: &'static str,
    high: &'static str,
    min: u8,
    max: u8,
    value: u8,
}

struct ToggleView {
    name: &'static str,
    label: &'static str,
    checked: bool,
}

impl LineupTemplate {
    fn build(team: &Team, players: &[&Player], error: String, saved: bool) -> Self {
        let mode = team.mode();
        let mut auto_team = team.clone();
        auto_team.lineup_mode = Some(LineupMode::Auto);
        let auto = Rotation::new(&auto_team, players);

        // Depth order: Auto shows the coach's own order; otherwise the
        // owner's saved starters then bench order, then anyone unlisted.
        let mut order: Vec<usize> = Vec::new();
        if mode == LineupMode::Auto {
            order.extend(auto.starters.iter().copied());
            order.extend(
                auto.ranked()
                    .iter()
                    .copied()
                    .filter(|i| !auto.starters.contains(i)),
            );
        } else {
            for id in team.starters.iter().chain(&team.bench_order) {
                if let Some(index) = players.iter().position(|p| &p.id == id)
                    && !order.contains(&index)
                {
                    order.push(index);
                }
            }
            for index in auto.ranked() {
                if !order.contains(index) {
                    order.push(*index);
                }
            }
        }

        // Chart to show: the saved one if valid, else the coach's suggestion.
        let saved_chart = valid_saved_chart(team, players);
        let chart_is_saved = saved_chart.is_some();
        let chart = saved_chart.unwrap_or_else(|| suggest_chart(team, players));
        let block_counts: Vec<u16> = (0..players.len())
            .map(|i| {
                chart.iter().filter(|block| block.contains(&i)).count() as u16 * CHART_BLOCK_MINUTES
            })
            .collect();

        let rows = order
            .iter()
            .enumerate()
            .map(|(slot, index)| {
                let player = players[*index];
                let saved_minutes = team.minute_targets.get(&player.id);
                DepthRow {
                    id: player.id.clone(),
                    name: player.name.clone(),
                    position: position_name(player.position),
                    age: player.age,
                    overall: player_overall(player),
                    endurance: player.ratings.endurance,
                    minutes: match (mode, saved_minutes) {
                        (LineupMode::Minutes, Some(minutes)) => minutes.to_string(),
                        _ => format!("{:.0}", auto.targets[*index] / 60.0),
                    },
                    starter: slot < 5,
                }
            })
            .collect();

        let chart_rows = order
            .iter()
            .map(|index| {
                let player = players[*index];
                ChartRow {
                    id: player.id.clone(),
                    name: player.name.clone(),
                    short_name: short_name(&player.name),
                    position: position_name(player.position),
                    cells: (0..CHART_BLOCKS)
                        .map(|block| ChartCell {
                            block,
                            on: chart[block].contains(index),
                        })
                        .collect(),
                    minutes: block_counts[*index],
                }
            })
            .collect();

        let s = team.strategy;
        let c = team.coach;
        Self {
            team_id: team.id.clone(),
            team_name: format!("{} {}", team.city, team.name),
            error,
            saved,
            mode: match mode {
                LineupMode::Auto => "auto",
                LineupMode::Minutes => "minutes",
                LineupMode::Chart => "chart",
            }
            .to_string(),
            rows,
            chart_rows,
            quarters: (1..=4)
                .map(|q| QuarterHead {
                    label: format!("Q{q}"),
                })
                .collect(),
            block_numbers: (0..CHART_BLOCKS).collect(),
            offense: vec![
                slider(
                    "pace",
                    "Pace",
                    "Slow, half-court",
                    "Run and gun",
                    0,
                    100,
                    s.pace,
                ),
                slider(
                    "three_rate",
                    "Three-point rate",
                    "Mid-range / paint",
                    "Chuck threes",
                    0,
                    100,
                    s.three_rate,
                ),
                slider(
                    "ball_movement",
                    "Ball movement",
                    "Isolation, star-heavy",
                    "Share the ball",
                    0,
                    100,
                    s.ball_movement,
                ),
                slider(
                    "offensive_glass",
                    "Offensive glass",
                    "Get back on D",
                    "Crash the boards",
                    0,
                    100,
                    s.offensive_glass,
                ),
            ],
            defense: vec![
                slider(
                    "pressure",
                    "Pressure",
                    "Stay home, no gambles",
                    "Gamble for steals",
                    0,
                    100,
                    s.pressure,
                ),
                slider(
                    "interior_focus",
                    "Defensive focus",
                    "Guard the arc",
                    "Pack the paint",
                    0,
                    100,
                    s.interior_focus,
                ),
                slider(
                    "defensive_glass",
                    "Defensive glass",
                    "Leak out in transition",
                    "Crash the glass",
                    0,
                    100,
                    s.defensive_glass,
                ),
            ],
            coach: vec![
                slider(
                    "foul_caution",
                    "Foul trouble",
                    "Let them play",
                    "Pull early",
                    0,
                    100,
                    c.foul_caution,
                ),
                slider(
                    "starter_load",
                    "Starter minutes",
                    "Spread minutes (~28)",
                    "Ride the starters (~38)",
                    0,
                    100,
                    c.starter_load,
                ),
                slider(
                    "fatigue_tolerance",
                    "Fatigue tolerance",
                    "Sub at first sign of tired",
                    "Play through it",
                    0,
                    100,
                    c.fatigue_tolerance,
                ),
                slider(
                    "depth",
                    "Rotation depth",
                    "7 players",
                    "12 players",
                    7,
                    12,
                    c.depth,
                ),
            ],
            toggles: vec![
                ToggleView {
                    name: "closers",
                    label: "Keep the best five in late in close games",
                    checked: c.closers,
                },
                ToggleView {
                    name: "blowout_bench",
                    label: "Rest starters in blowouts",
                    checked: c.blowout_bench,
                },
                ToggleView {
                    name: "stagger_stars",
                    label: "Stagger stars (Auto mode)",
                    checked: c.stagger_stars,
                },
            ],
            chart_is_saved,
        }
    }
}

fn slider(
    name: &'static str,
    label: &'static str,
    low: &'static str,
    high: &'static str,
    min: u8,
    max: u8,
    value: u8,
) -> SliderView {
    SliderView {
        name,
        label,
        low,
        high,
        min,
        max,
        value,
    }
}

fn valid_saved_chart(team: &Team, players: &[&Player]) -> Option<Vec<Vec<usize>>> {
    if team.chart.len() != CHART_BLOCKS {
        return None;
    }
    team.chart
        .iter()
        .map(|block| {
            let indices: Option<Vec<usize>> = block
                .iter()
                .map(|id| players.iter().position(|p| &p.id == id))
                .collect();
            indices.filter(|indices| {
                let mut unique = indices.clone();
                unique.sort_unstable();
                unique.dedup();
                indices.len() == 5 && unique.len() == 5
            })
        })
        .collect()
}

/// "Kai Dawson" -> "K. Dawson" for tight chart rows.
fn short_name(name: &str) -> String {
    let mut parts = name.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(first), Some(last)) => {
            format!("{}. {}", first.chars().next().unwrap_or('?'), last)
        }
        _ => name.to_string(),
    }
}
