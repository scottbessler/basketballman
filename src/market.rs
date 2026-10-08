//! Web pages and actions for free agency, the draft room, the offseason
//! cycle and creating a new league.

use crate::config::{DRAFT_ROUNDS, FANTASY_ROUNDS, MIN_SALARY, ROSTER_MAX, ROSTER_MIN, SALARY_CAP};
use crate::contracts::{
    ask_salary, fmt_m, free_agents, market_salary, payroll, preferred_years, resign_player,
    sign_free_agent, team_label, waive_player,
};
use crate::draft::{
    auto_pick, available_players, make_pick, run_ai_until_human, run_all, scouted_overall,
    scouted_potential, would_overspend,
};
use crate::generator::{StartMode, generate_league_with};
use crate::models::{DraftKind, League, Phase, Player, PlayerStatus};
use crate::offseason::{advance_phase, next_step_label, sim_free_agency_days};
use crate::playoffs::champion;
use crate::routes::{AppState, owned_team_id, persist_and_redirect, position_name, render};
use crate::session::{AuthUser, MaybeUser};
use crate::sim::player_overall;
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Flash {
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub message: String,
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

pub fn flash_path(path: &str, kind: &str, text: &str) -> String {
    format!("{path}?{kind}={}", percent_encode(text))
}

fn flash(path: &str, kind: &str, text: &str) -> Response {
    Redirect::to(&flash_path(path, kind, text)).into_response()
}

fn not_signed_in_team() -> &'static str {
    "Claim a team first."
}

const YEARS: [u8; 5] = [1, 2, 3, 4, 5];

// ---------------------------------------------------------------- free agents

#[derive(Template)]
#[template(path = "free_agents.html")]
struct FreeAgentsTemplate {
    phase: String,
    fa_day: u16,
    error: String,
    message: String,
    has_team: bool,
    team_name: String,
    payroll: String,
    room: String,
    cap: String,
    roster_len: usize,
    roster_max: usize,
    can_sign: bool,
    rows: Vec<FreeAgentRow>,
}

struct YearOption {
    value: u8,
    selected: bool,
}

fn year_options(preferred: u8) -> Vec<YearOption> {
    YEARS
        .iter()
        .map(|value| YearOption {
            value: *value,
            selected: *value == preferred,
        })
        .collect()
}

struct FreeAgentRow {
    id: String,
    name: String,
    position: String,
    age: u8,
    overall: u16,
    potential: u16,
    endurance: u8,
    ask: String,
    options: Vec<YearOption>,
    affordable: bool,
}

pub async fn free_agents_page(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Query(flash): Query<Flash>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    let team_id = viewer.and_then(|user| owned_team_id(&league, user));
    let (roster_len, paid) = team_id
        .as_ref()
        .and_then(|id| league.teams.iter().find(|t| &t.id == id))
        .map(|team| (team.roster.len(), payroll(&league, &team.id)))
        .unwrap_or((0, 0));
    let room = SALARY_CAP.saturating_sub(paid);
    let can_sign = team_id.is_some() && league.phase != Phase::FantasyDraft;
    let rows = free_agents(&league)
        .into_iter()
        .take(150)
        .map(|player| {
            let ask = ask_salary(player, league.fa_day);
            FreeAgentRow {
                id: player.id.clone(),
                name: player.name.clone(),
                position: position_name(player.position),
                age: player.age,
                overall: player_overall(player),
                potential: scouted_potential(player),
                endurance: player.ratings.endurance,
                ask: fmt_m(ask),
                options: year_options(preferred_years(player)),
                affordable: roster_len < ROSTER_MAX
                    && (ask <= room || (ask == MIN_SALARY && roster_len < ROSTER_MIN)),
            }
        })
        .collect();
    render(FreeAgentsTemplate {
        phase: league.phase.to_string(),
        fa_day: league.fa_day,
        error: flash.error,
        message: flash.message,
        has_team: team_id.is_some(),
        team_name: team_id
            .as_deref()
            .map(|id| team_label(&league, id))
            .unwrap_or_default(),
        payroll: fmt_m(paid),
        room: fmt_m(room),
        cap: fmt_m(SALARY_CAP),
        roster_len,
        roster_max: ROSTER_MAX,
        can_sign,
        rows,
    })
}

#[derive(Deserialize)]
pub struct YearsForm {
    #[serde(default = "default_years")]
    years: u8,
}

fn default_years() -> u8 {
    2
}

pub async fn sign(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(player_id): Path<String>,
    Form(form): Form<YearsForm>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(team_id) = owned_team_id(&league, user_id) else {
        return flash("/free-agents", "error", not_signed_in_team());
    };
    if league.phase == Phase::FantasyDraft {
        return flash(
            "/free-agents",
            "error",
            "Free agency opens after the fantasy draft.",
        );
    }
    match sign_free_agent(&mut league, &team_id, &player_id, form.years) {
        Ok(contract) => {
            let name = league
                .players
                .iter()
                .find(|p| p.id == player_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let target = flash_path(
                "/free-agents",
                "message",
                &format!(
                    "Signed {name} for ${}M x {}y.",
                    fmt_m(contract.salary),
                    contract.years_left
                ),
            );
            persist_and_redirect(&state, &league, &target)
        }
        Err(error) => flash("/free-agents", "error", &error.to_string()),
    }
}

pub async fn waive(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path((team_id, player_id)): Path<(String, String)>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let owns = league
        .teams
        .iter()
        .any(|team| team.id == team_id && team.owner_user_id == Some(user_id));
    if !owns {
        return (StatusCode::FORBIDDEN, "you do not manage this team").into_response();
    }
    let path = format!("/teams/{team_id}");
    match waive_player(&mut league, &team_id, &player_id) {
        Ok(()) => persist_and_redirect(
            &state,
            &league,
            &flash_path(&path, "message", "Player waived."),
        ),
        Err(error) => flash(&path, "error", &error.to_string()),
    }
}

pub async fn resign(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path((team_id, player_id)): Path<(String, String)>,
    Form(form): Form<YearsForm>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let owns = league
        .teams
        .iter()
        .any(|team| team.id == team_id && team.owner_user_id == Some(user_id));
    if !owns {
        return (StatusCode::FORBIDDEN, "you do not manage this team").into_response();
    }
    match resign_player(&mut league, &team_id, &player_id, form.years) {
        Ok(contract) => persist_and_redirect(
            &state,
            &league,
            &flash_path(
                "/offseason",
                "message",
                &format!(
                    "Re-signed for ${}M x {}y.",
                    fmt_m(contract.salary),
                    contract.years_left
                ),
            ),
        ),
        Err(error) => flash("/offseason", "error", &error.to_string()),
    }
}

// ----------------------------------------------------------------- draft room

#[derive(Template)]
#[template(path = "draft.html")]
struct DraftTemplate {
    title: String,
    eyebrow: String,
    error: String,
    message: String,
    active: bool,
    complete: bool,
    pick_number: usize,
    total_picks: usize,
    clock_team: String,
    has_team: bool,
    my_turn: bool,
    cap_note: String,
    upcoming: Vec<ClockSlot>,
    scouted: bool,
    fantasy: bool,
    can_pick: bool,
    sort_index: usize,
    board: Vec<BoardRow>,
    has_picks: bool,
    picks: Vec<PickRow>,
}

struct ClockSlot {
    number: usize,
    team: String,
    mine: bool,
}

struct BoardRow {
    id: String,
    name: String,
    position: String,
    age: u8,
    overall: u16,
    potential: u16,
    endurance: u8,
    salary: String,
    affordable: bool,
}

struct PickRow {
    number: u16,
    team_id: String,
    team: String,
    player_id: String,
    player: String,
    position: String,
    age: u8,
    overall: u16,
    potential: u16,
}

fn board_row(league: &League, player: &Player, my_team: Option<&str>, fantasy: bool) -> BoardRow {
    let salary = market_salary(player);
    BoardRow {
        id: player.id.clone(),
        name: player.name.clone(),
        position: position_name(player.position),
        age: player.age,
        overall: scouted_overall(player),
        potential: scouted_potential(player),
        endurance: player.ratings.endurance,
        salary: fmt_m(salary),
        affordable: !fantasy || my_team.is_none_or(|team| !would_overspend(league, team, salary)),
    }
}

pub async fn draft_page(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Query(flash): Query<Flash>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    let my_team = viewer.and_then(|user| owned_team_id(&league, user));
    let draft = league.draft.as_ref();
    let fantasy = draft.is_some_and(|d| d.kind == DraftKind::Fantasy);
    let clock_team_id = draft.and_then(|d| d.on_the_clock().cloned());
    let my_turn = my_team.is_some() && my_team == clock_team_id;
    let complete = draft.is_some_and(|d| d.complete());

    let pool: Vec<&Player> = if draft.is_some() {
        available_players(&league)
    } else {
        let mut upcoming: Vec<&Player> = league
            .players
            .iter()
            .filter(|p| p.status == PlayerStatus::Prospect && p.draft_year == Some(league.season))
            .collect();
        upcoming.sort_by_key(|p| {
            std::cmp::Reverse(scouted_overall(p) as u32 * 45 + scouted_potential(p) as u32 * 55)
        });
        upcoming
    };
    let board: Vec<BoardRow> = pool
        .into_iter()
        .take(200)
        .map(|player| board_row(&league, player, my_team.as_deref(), fantasy))
        .collect();

    let (pick_number, total_picks) = draft
        .map(|d| ((d.picks.len() + 1).min(d.order.len()), d.order.len()))
        .unwrap_or((0, 0));
    let upcoming = draft
        .map(|d| {
            d.order
                .iter()
                .enumerate()
                .skip(d.picks.len())
                .take(12)
                .map(|(index, team_id)| ClockSlot {
                    number: index + 1,
                    team: team_label(&league, team_id),
                    mine: my_team.as_ref() == Some(team_id),
                })
                .collect()
        })
        .unwrap_or_default();
    let picks: Vec<PickRow> = draft
        .map(|d| {
            d.picks
                .iter()
                .rev()
                .take(80)
                .filter_map(|pick| {
                    let player = league.players.iter().find(|p| p.id == pick.player_id)?;
                    Some(PickRow {
                        number: pick.number,
                        team_id: pick.team_id.clone(),
                        team: team_label(&league, &pick.team_id),
                        player_id: player.id.clone(),
                        player: player.name.clone(),
                        position: position_name(player.position),
                        age: player.age,
                        overall: player_overall(player),
                        potential: player.potential.max(player_overall(player) as u8) as u16,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let (title, eyebrow) = match (draft.map(|d| d.kind), league.phase) {
        (Some(DraftKind::Fantasy), _) => (
            "Fantasy Draft".to_string(),
            format!("Snake draft, {FANTASY_ROUNDS} rounds"),
        ),
        (Some(DraftKind::Rookie), _) => (
            format!("{} Draft", league.season),
            format!("{DRAFT_ROUNDS} rounds, lottery order"),
        ),
        (None, _) => (
            format!("{} Draft Class", league.season),
            "Scouting report".to_string(),
        ),
    };
    let cap_note = if my_turn && fantasy {
        my_team
            .as_deref()
            .map(|id| {
                format!(
                    "cap room ${}M",
                    fmt_m(SALARY_CAP.saturating_sub(payroll(&league, id)))
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let can_pick = my_turn && !complete;
    render(DraftTemplate {
        title,
        eyebrow,
        error: flash.error,
        message: flash.message,
        active: draft.is_some(),
        complete,
        pick_number,
        total_picks,
        clock_team: clock_team_id
            .as_deref()
            .map(|id| team_label(&league, id))
            .unwrap_or_default(),
        has_team: my_team.is_some(),
        my_turn,
        cap_note,
        upcoming,
        scouted: !fantasy,
        fantasy,
        can_pick,
        sort_index: 3,
        board,
        has_picks: !picks.is_empty(),
        picks,
    })
}

pub async fn draft_pick(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(player_id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(team_id) = owned_team_id(&league, user_id) else {
        return flash("/draft", "error", not_signed_in_team());
    };
    match make_pick(&mut league, &team_id, &player_id) {
        Ok(pick) => persist_and_redirect(
            &state,
            &league,
            &flash_path("/draft", "message", &format!("Pick #{} made.", pick.number)),
        ),
        Err(error) => flash("/draft", "error", &error.to_string()),
    }
}

pub async fn draft_auto(State(state): State<AppState>, AuthUser(user_id): AuthUser) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(team_id) = owned_team_id(&league, user_id) else {
        return flash("/draft", "error", not_signed_in_team());
    };
    let on_clock = league
        .draft
        .as_ref()
        .and_then(|draft| draft.on_the_clock().cloned());
    if on_clock.as_ref() != Some(&team_id) {
        return flash("/draft", "error", "Your team is not on the clock.");
    }
    match auto_pick(&mut league) {
        Ok(pick) => persist_and_redirect(
            &state,
            &league,
            &flash_path(
                "/draft",
                "message",
                &format!("Auto-picked #{}.", pick.number),
            ),
        ),
        Err(error) => flash("/draft", "error", &error.to_string()),
    }
}

pub async fn draft_sim(State(state): State<AppState>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let made = run_ai_until_human(&mut league);
    persist_and_redirect(
        &state,
        &league,
        &flash_path("/draft", "message", &format!("{made} picks made.")),
    )
}

pub async fn draft_sim_all(State(state): State<AppState>, AuthUser(_): AuthUser) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let made = run_all(&mut league);
    persist_and_redirect(
        &state,
        &league,
        &flash_path("/draft", "message", &format!("{made} picks made.")),
    )
}

// ------------------------------------------------------------------ offseason

#[derive(Template)]
#[template(path = "offseason.html")]
struct OffseasonTemplate {
    season: u16,
    phase: String,
    phase_blurb: String,
    error: String,
    message: String,
    steps: Vec<StepView>,
    has_next: bool,
    next_label: String,
    free_agency: bool,
    has_expiring: bool,
    can_resign: bool,
    team_id: String,
    room: String,
    expiring: Vec<ExpiringRow>,
    history: Vec<HistoryRow>,
    transactions: Vec<String>,
}

struct StepView {
    name: &'static str,
    current: bool,
}

struct ExpiringRow {
    id: String,
    name: String,
    age: u8,
    overall: u16,
    potential: u16,
    ask: String,
    options: Vec<YearOption>,
    affordable: bool,
}

struct HistoryRow {
    season: u16,
    champion: String,
    best_record: String,
    scoring_leader: String,
}

pub async fn offseason_page(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Query(flash): Query<Flash>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    let my_team = viewer.and_then(|user| owned_team_id(&league, user));
    let current = match league.phase {
        Phase::RegularSeason => 0,
        Phase::FantasyDraft | Phase::Draft => 1,
        Phase::Resign => 2,
        Phase::FreeAgency => 3,
    };
    let steps = ["Season & playoffs", "Draft", "Re-signing", "Free agency"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| StepView {
            name,
            current: index == current,
        })
        .collect();
    let blurb = match league.phase {
        Phase::FantasyDraft => "The fantasy draft is building the league.".to_string(),
        Phase::RegularSeason if champion(&league).is_some() => {
            "The season is over. Start the draft to begin the offseason.".to_string()
        }
        Phase::RegularSeason => "The season is underway.".to_string(),
        Phase::Draft => "The rookie draft is in progress.".to_string(),
        Phase::Resign => {
            "Players have developed (or declined). Re-sign expiring players before they hit the market.".to_string()
        }
        Phase::FreeAgency => format!(
            "Free agency, day {}. Asking prices fall as days pass; AI teams are shopping.",
            league.fa_day
        ),
    };
    let expiring: Vec<ExpiringRow> = match (&my_team, league.phase) {
        (Some(team_id), Phase::Resign) => league
            .teams
            .iter()
            .find(|team| &team.id == team_id)
            .map(|team| {
                let room = SALARY_CAP.saturating_sub(payroll(&league, team_id));
                team.roster
                    .iter()
                    .filter_map(|id| league.players.iter().find(|p| &p.id == id))
                    .filter(|p| p.contract.is_some_and(|c| c.years_left == 0))
                    .map(|p| {
                        let ask = market_salary(p);
                        ExpiringRow {
                            id: p.id.clone(),
                            name: p.name.clone(),
                            age: p.age,
                            overall: player_overall(p),
                            potential: scouted_potential(p),
                            ask: fmt_m(ask),
                            options: year_options(preferred_years(p)),
                            affordable: ask <= room,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let room = my_team
        .as_ref()
        .map(|id| SALARY_CAP.saturating_sub(payroll(&league, id)))
        .unwrap_or(0);
    let history = league
        .history
        .iter()
        .rev()
        .map(|row| HistoryRow {
            season: row.season,
            champion: row.champion_name.clone(),
            best_record: row.best_record.clone(),
            scoring_leader: row.scoring_leader.clone(),
        })
        .collect();
    let label = next_step_label(&league);
    render(OffseasonTemplate {
        season: league.season,
        phase: league.phase.to_string(),
        phase_blurb: blurb,
        error: flash.error,
        message: flash.message,
        steps,
        has_next: label.is_some(),
        next_label: label.unwrap_or_default().to_string(),
        free_agency: league.phase == Phase::FreeAgency,
        has_expiring: !expiring.is_empty(),
        can_resign: league.phase == Phase::Resign,
        team_id: my_team.clone().unwrap_or_default(),
        room: fmt_m(room),
        expiring,
        history,
        transactions: league.transactions.iter().rev().take(60).cloned().collect(),
    })
}

pub async fn offseason_advance(State(state): State<AppState>, AuthUser(_): AuthUser) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let target = match advance_phase(&mut league) {
        Ok(message) => match league.phase {
            Phase::Draft => "/draft".to_string(),
            _ => flash_path("/offseason", "message", &message),
        },
        Err(error) => return flash("/offseason", "error", &error),
    };
    persist_and_redirect(&state, &league, &target)
}

#[derive(Deserialize)]
pub struct DaysForm {
    #[serde(default = "default_days")]
    days: u16,
}

fn default_days() -> u16 {
    1
}

pub async fn offseason_fa_days(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Form(form): Form<DaysForm>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    if league.phase != Phase::FreeAgency {
        return flash("/offseason", "error", "Free agency is not open.");
    }
    sim_free_agency_days(&mut league, form.days.clamp(1, 30));
    persist_and_redirect(
        &state,
        &league,
        &flash_path(
            "/offseason",
            "message",
            &format!("{} days simulated.", form.days.clamp(1, 30)),
        ),
    )
}

// ----------------------------------------------------------------- new league

#[derive(Template)]
#[template(path = "league_new.html")]
struct NewLeagueTemplate {
    error: String,
    rounds: usize,
}

#[derive(Deserialize)]
pub struct NewLeagueForm {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    seed: String,
}

pub async fn new_league_page(Query(flash): Query<Flash>) -> Response {
    render(NewLeagueTemplate {
        error: flash.error,
        rounds: FANTASY_ROUNDS,
    })
}

pub async fn new_league(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Form(form): Form<NewLeagueForm>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let seed = match form.seed.trim() {
        "" => league.seed.wrapping_add(1),
        text => match text.parse::<u64>() {
            Ok(seed) => seed,
            Err(_) => return flash("/league/new", "error", "Seed must be a whole number."),
        },
    };
    let fantasy = form.mode == "fantasy";
    let next = generate_league_with(
        seed,
        if fantasy {
            StartMode::Fantasy
        } else {
            StartMode::Quick
        },
    );
    *league = next;
    let target = if fantasy { "/draft" } else { "/standings" };
    persist_and_redirect(&state, &league, target)
}
