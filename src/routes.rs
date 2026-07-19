use crate::auth;
use crate::models::{
    Conference, Game, GameResult, GameStatus, League, Player, PlayerGameStats, PlayerSeasonStats,
    Position, Team, TradeStatus,
};
use crate::playoffs::{
    REGULAR_SEASON_DATES, advance_playoff_day, champion, regular_season_complete, start_playoffs,
};
use crate::repo::LeagueRepository;
use crate::session::{AuthUser, MaybeUser};
use crate::sim::{SimConfig, player_overall, simulate_game, team_rating};
use crate::stats::{next_unplayed_date_indices, player_season_stats, standings};
use crate::trades;
use crate::users::UserStore;
use askama::Template;
use axum::Router;
use axum::extract::{Form, FromRef, Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_extra::extract::cookie::Key;
use serde::Deserialize;
use std::cmp::Reverse;
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;
use uuid::Uuid;
use webauthn_rs::prelude::Webauthn;

#[derive(Clone)]
pub struct AppState {
    pub repo: LeagueRepository,
    pub league: Arc<Mutex<League>>,
    pub users: Arc<UserStore>,
    pub webauthn: Arc<Webauthn>,
    pub key: Key,
    /// When set, auth skips the WebAuthn ceremony and trusts the username
    /// alone. Dev-only escape hatch — browsers dislike passkeys on localhost.
    pub passkey_disabled: bool,
}

/// Lets the signed-cookie extractors pull the signing key out of `AppState`.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.key.clone()
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthcheck", get(healthcheck))
        .route("/login", get(login_page))
        .route("/auth/register/begin", post(auth::register_begin))
        .route("/auth/register/finish", post(auth::register_finish))
        .route("/auth/login/begin", post(auth::login_begin))
        .route("/auth/login/finish", post(auth::login_finish))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(my_team))
        .route("/standings", get(standings_page))
        .route("/teams", get(teams))
        .route("/teams/{id}", get(team_detail))
        .route("/teams/{id}/claim", post(claim_team))
        .route("/teams/{id}/release", post(release_team))
        .route("/teams/{id}/lineup", post(set_lineup))
        .route("/players/{id}", get(player_detail))
        .route("/schedule", get(schedule))
        .route("/games/{id}", get(game_detail))
        .route("/games/{id}/simulate", post(simulate))
        .route("/sim/day", post(sim_day))
        .route("/sim/week", post(sim_week))
        .route("/sim/month", post(sim_month))
        .route("/trades", get(trades_page).post(create_trade))
        .route("/trades/new", get(trade_new_page))
        .route("/trades/{id}/accept", post(accept_trade_route))
        .route("/trades/{id}/reject", post(reject_trade_route))
        .route("/trades/{id}/withdraw", post(withdraw_trade_route))
        .route("/playoffs", get(playoffs_page))
        .route("/playoffs/start", post(start_playoffs_route))
        .route("/playoffs/sim", post(sim_playoff_day))
        .route("/league/reset", post(reset_league))
        .route("/league/regen", post(regen_league))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
}

async fn healthcheck() -> &'static str {
    "OK"
}

async fn index(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(IndexTemplate::from_league(&league))
}

async fn standings_page(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(StandingsTemplate::from_league(&league))
}

async fn teams(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(TeamsTemplate::from_league(&league))
}

async fn team_detail(
    State(state): State<AppState>,
    MaybeUser(viewer): MaybeUser,
    Path(id): Path<String>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    match TeamTemplate::from_league(&league, &state.users, viewer, &id) {
        Some(template) => render(template),
        None => (axum::http::StatusCode::NOT_FOUND, "team not found").into_response(),
    }
}

async fn login_page() -> Response {
    render(LoginTemplate {})
}

/// Landing spot for the signed-in owner: their team page, or the team list so
/// they can claim one.
async fn my_team(State(state): State<AppState>, AuthUser(user_id): AuthUser) -> Redirect {
    let league = state.league.lock().expect("league lock");
    match owned_team_id(&league, user_id) {
        Some(team_id) => Redirect::to(&format!("/teams/{team_id}")),
        None => Redirect::to("/teams"),
    }
}

async fn claim_team(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    if owned_team_id(&league, user_id).is_some() {
        return (
            axum::http::StatusCode::CONFLICT,
            "you already manage a team",
        )
            .into_response();
    }
    let Some(team) = league.teams.iter_mut().find(|team| team.id == id) else {
        return (axum::http::StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if team.owner_user_id.is_some() {
        return (
            axum::http::StatusCode::CONFLICT,
            "that team already has an owner",
        )
            .into_response();
    }
    team.owner_user_id = Some(user_id);
    persist_and_redirect(&state, &league, &format!("/teams/{id}"))
}

async fn release_team(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(team) = league.teams.iter_mut().find(|team| team.id == id) else {
        return (axum::http::StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if team.owner_user_id != Some(user_id) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "you do not manage this team",
        )
            .into_response();
    }
    team.owner_user_id = None;
    team.starters.clear();
    team.minute_targets.clear();
    persist_and_redirect(&state, &league, &format!("/teams/{id}"))
}

/// Save starters and minute targets. The form posts repeated `starter`
/// checkboxes plus one `min_<player_id>` field per roster player.
async fn set_lineup(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(team) = league.teams.iter().find(|team| team.id == id) else {
        return (axum::http::StatusCode::NOT_FOUND, "team not found").into_response();
    };
    if team.owner_user_id != Some(user_id) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "you do not manage this team",
        )
            .into_response();
    }
    let roster = team.roster.clone();
    let mut starters: Vec<String> = Vec::new();
    let mut minute_targets = std::collections::BTreeMap::new();
    for (name, value) in &fields {
        if name == "starter" && roster.contains(value) && !starters.contains(value) {
            starters.push(value.clone());
        } else if let Some(player_id) = name.strip_prefix("min_")
            && roster.iter().any(|id| id == player_id)
            && let Ok(minutes) = value.trim().parse::<u16>()
        {
            minute_targets.insert(player_id.to_string(), minutes.min(48));
        }
    }
    if starters.len() != 5 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "pick exactly 5 starters",
        )
            .into_response();
    }
    let team = league
        .teams
        .iter_mut()
        .find(|team| team.id == id)
        .expect("team");
    team.starters = starters;
    team.minute_targets = minute_targets;
    persist_and_redirect(&state, &league, &format!("/teams/{id}"))
}

async fn player_detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    match PlayerTemplate::from_league(&league, &id) {
        Some(template) => render(template),
        None => (axum::http::StatusCode::NOT_FOUND, "player not found").into_response(),
    }
}

async fn schedule(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(ScheduleTemplate::from_league(&league))
}

async fn game_detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    match GameTemplate::from_league(&league, &id) {
        Some(template) => render(template),
        None => (axum::http::StatusCode::NOT_FOUND, "game not found").into_response(),
    }
}

async fn simulate(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let game_exists = league.schedule.iter().any(|game| game.id == id);
    if !game_exists {
        return (axum::http::StatusCode::NOT_FOUND, "game not found").into_response();
    }
    {
        simulate_game(&mut league, &id, SimConfig::default());
    }
    if let Err(error) = state.repo.save(&league) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("save failed: {error}"),
        )
            .into_response();
    }
    Redirect::to(&format!("/games/{id}")).into_response()
}

async fn sim_day(State(state): State<AppState>) -> Response {
    persist_simulation(state, |league| simulate_next_dates(league, 1))
}

async fn sim_week(State(state): State<AppState>) -> Response {
    persist_simulation(state, |league| simulate_next_dates(league, 7))
}

async fn sim_month(State(state): State<AppState>) -> Response {
    persist_simulation(state, |league| simulate_next_dates(league, 30))
}

async fn reset_league(State(state): State<AppState>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    if let Err(error) = state.repo.reset(&mut league) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("reset failed: {error}"),
        )
            .into_response();
    }
    Redirect::to("/standings").into_response()
}

async fn regen_league(State(state): State<AppState>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    match state.repo.regenerate(league.seed) {
        Ok(next) => {
            *league = next;
            Redirect::to("/standings").into_response()
        }
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("regen failed: {error}"),
        )
            .into_response(),
    }
}

fn persist_simulation(state: AppState, mutate: impl FnOnce(&mut League)) -> Response {
    let mut league = state.league.lock().expect("league lock");
    mutate(&mut league);
    if let Err(error) = state.repo.save(&league) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("save failed: {error}"),
        )
            .into_response();
    }
    Redirect::to("/standings").into_response()
}

fn simulate_next_dates(league: &mut League, date_count: usize) {
    let dates = next_unplayed_date_indices(league, date_count);
    let game_ids: Vec<String> = league
        .schedule
        .iter()
        .filter(|game| game.status == GameStatus::Scheduled && dates.contains(&game.date_index))
        .map(|game| game.id.clone())
        .collect();
    for game_id in game_ids {
        simulate_game(league, &game_id, SimConfig::default());
    }
    // Once the regular season is over, the sim buttons roll the playoffs
    // forward one date per remaining day.
    let mut remaining = date_count.saturating_sub(dates.len());
    while remaining > 0 && advance_playoff_day(league, SimConfig::default()) {
        remaining -= 1;
    }
}

fn persist_and_redirect(state: &AppState, league: &League, to: &str) -> Response {
    if let Err(error) = state.repo.save(league) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("save failed: {error}"),
        )
            .into_response();
    }
    Redirect::to(to).into_response()
}

fn owned_team_id(league: &League, user_id: Uuid) -> Option<String> {
    league
        .teams
        .iter()
        .find(|team| team.owner_user_id == Some(user_id))
        .map(|team| team.id.clone())
}

fn team_label(league: &League, team_id: &str) -> String {
    league
        .teams
        .iter()
        .find(|team| team.id == team_id)
        .map(|team| format!("{} {}", team.city, team.name))
        .unwrap_or_else(|| team_id.to_string())
}

fn player_names(league: &League, player_ids: &[String]) -> String {
    player_ids
        .iter()
        .map(|player_id| {
            league
                .players
                .iter()
                .find(|player| &player.id == player_id)
                .map(|player| player.name.clone())
                .unwrap_or_else(|| player_id.clone())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Deserialize)]
struct TradeNewQuery {
    team: Option<String>,
    counter_of: Option<String>,
}

async fn trades_page(State(state): State<AppState>, MaybeUser(viewer): MaybeUser) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(TradesTemplate::from_league(&league, viewer))
}

async fn trade_new_page(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Query(query): Query<TradeNewQuery>,
) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    let Some(my_team_id) = owned_team_id(&league, user_id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "claim a team before proposing trades",
        )
            .into_response();
    };
    // Countering swaps the direction: the target becomes the original sender.
    let (to_team_id, counter_of) = if let Some(counter_id) = query.counter_of {
        let Some(original) = league.trades.iter().find(|trade| trade.id == counter_id) else {
            return (axum::http::StatusCode::NOT_FOUND, "trade not found").into_response();
        };
        if original.to_team_id != my_team_id || original.status != TradeStatus::Pending {
            return (
                axum::http::StatusCode::FORBIDDEN,
                "you can only counter pending offers sent to your team",
            )
                .into_response();
        }
        (original.from_team_id.clone(), Some(counter_id))
    } else {
        match query.team {
            Some(team) => (team, None),
            None => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    "pick a team to trade with",
                )
                    .into_response();
            }
        }
    };
    match TradeNewTemplate::from_league(&league, &my_team_id, &to_team_id, counter_of) {
        Some(template) => render(template),
        None => (axum::http::StatusCode::NOT_FOUND, "team not found").into_response(),
    }
}

async fn create_trade(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(from_team_id) = owned_team_id(&league, user_id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "claim a team before proposing trades",
        )
            .into_response();
    };
    let mut to_team_id = String::new();
    let mut offered: Vec<String> = Vec::new();
    let mut requested: Vec<String> = Vec::new();
    let mut note = None;
    let mut counter_of = None;
    for (name, value) in &fields {
        match name.as_str() {
            "to_team" => to_team_id = value.clone(),
            "offer" if !offered.contains(value) => offered.push(value.clone()),
            "request" if !requested.contains(value) => requested.push(value.clone()),
            "note" => note = trades::clean_note(value),
            "counter_of" if !value.is_empty() => counter_of = Some(value.clone()),
            _ => {}
        }
    }
    if let Err(error) =
        trades::validate_offer(&league, &from_team_id, &to_team_id, &offered, &requested)
    {
        return (axum::http::StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    // A counter closes out the offer it answers and carries its note there too.
    if let Some(counter_id) = &counter_of {
        let Some(original) = trades::trade_mut(&mut league, counter_id) else {
            return (axum::http::StatusCode::NOT_FOUND, "trade not found").into_response();
        };
        if original.to_team_id != from_team_id || original.status != TradeStatus::Pending {
            return (
                axum::http::StatusCode::FORBIDDEN,
                "you can only counter pending offers sent to your team",
            )
                .into_response();
        }
        original.status = TradeStatus::Countered;
        original.response_note = note.clone();
    }
    let trade = crate::models::TradeOffer {
        id: trades::next_trade_id(&league),
        from_team_id,
        to_team_id,
        offered_player_ids: offered,
        requested_player_ids: requested,
        note,
        status: TradeStatus::Pending,
        response_note: None,
        counter_of,
    };
    league.trades.push(trade);
    persist_and_redirect(&state, &league, "/trades")
}

async fn accept_trade_route(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let Some(trade) = league.trades.iter().find(|trade| trade.id == id) else {
        return (axum::http::StatusCode::NOT_FOUND, "trade not found").into_response();
    };
    if owned_team_id(&league, user_id).as_deref() != Some(trade.to_team_id.as_str()) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "only the receiving owner can accept",
        )
            .into_response();
    }
    if let Err(error) = trades::accept_trade(&mut league, &id) {
        return (axum::http::StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    persist_and_redirect(&state, &league, "/trades")
}

async fn reject_trade_route(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let my_team = owned_team_id(&league, user_id);
    let note = fields
        .iter()
        .find(|(name, _)| name == "note")
        .and_then(|(_, value)| trades::clean_note(value));
    let Some(trade) = trades::trade_mut(&mut league, &id) else {
        return (axum::http::StatusCode::NOT_FOUND, "trade not found").into_response();
    };
    if my_team.as_deref() != Some(trade.to_team_id.as_str()) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "only the receiving owner can reject",
        )
            .into_response();
    }
    if trade.status != TradeStatus::Pending {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "trade is no longer pending",
        )
            .into_response();
    }
    trade.status = TradeStatus::Rejected;
    trade.response_note = note;
    persist_and_redirect(&state, &league, "/trades")
}

async fn withdraw_trade_route(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Path(id): Path<String>,
) -> Response {
    let mut league = state.league.lock().expect("league lock");
    let my_team = owned_team_id(&league, user_id);
    let Some(trade) = trades::trade_mut(&mut league, &id) else {
        return (axum::http::StatusCode::NOT_FOUND, "trade not found").into_response();
    };
    if my_team.as_deref() != Some(trade.from_team_id.as_str()) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "only the offering owner can withdraw",
        )
            .into_response();
    }
    if trade.status != TradeStatus::Pending {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "trade is no longer pending",
        )
            .into_response();
    }
    trade.status = TradeStatus::Withdrawn;
    persist_and_redirect(&state, &league, "/trades")
}

async fn playoffs_page(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(PlayoffsTemplate::from_league(&league))
}

async fn start_playoffs_route(State(state): State<AppState>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    if !start_playoffs(&mut league) && league.playoffs.is_none() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "finish the regular season first",
        )
            .into_response();
    }
    persist_and_redirect(&state, &league, "/playoffs")
}

async fn sim_playoff_day(State(state): State<AppState>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    advance_playoff_day(&mut league, SimConfig::default());
    persist_and_redirect(&state, &league, "/playoffs")
}

fn render<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("render failed: {error}"),
        )
            .into_response(),
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    league_name: String,
    season: u16,
    teams: usize,
    players: usize,
    games: usize,
    played: usize,
    leaders: Vec<TeamRow>,
}

impl IndexTemplate {
    fn from_league(league: &League) -> Self {
        let mut leaders: Vec<TeamRow> = league
            .teams
            .iter()
            .map(|team| TeamRow::from_team(league, team))
            .collect();
        leaders.sort_by_key(|leader| Reverse(leader.rating));
        leaders.truncate(8);

        let regular_season: Vec<_> = league
            .schedule
            .iter()
            .filter(|game| game.date_index <= REGULAR_SEASON_DATES)
            .collect();
        Self {
            league_name: league.name.clone(),
            season: league.season,
            teams: league.teams.len(),
            players: league.players.len(),
            games: regular_season.len(),
            played: regular_season
                .iter()
                .filter(|game| game.status == GameStatus::Played)
                .count(),
            leaders,
        }
    }
}

#[derive(Template)]
#[template(path = "standings.html")]
struct StandingsTemplate {
    east: Vec<StandingRow>,
    west: Vec<StandingRow>,
    played: usize,
    games: usize,
    next_day: String,
}

impl StandingsTemplate {
    fn from_league(league: &League) -> Self {
        let records = standings(league);
        let mut rows: Vec<StandingRow> = league
            .teams
            .iter()
            .map(|team| {
                StandingRow::from_team(league, team, records.get(&team.id).expect("record"))
            })
            .collect();
        rows.sort_by(|a, b| {
            b.wins
                .cmp(&a.wins)
                .then_with(|| a.losses.cmp(&b.losses))
                .then_with(|| b.diff.cmp(&a.diff))
        });
        let east = rows
            .iter()
            .filter(|row| row.conference == "East")
            .cloned()
            .collect();
        let west = rows
            .into_iter()
            .filter(|row| row.conference == "West")
            .collect();
        let next_day = next_unplayed_date_indices(league, 1)
            .first()
            .map(|day| day.to_string())
            .unwrap_or_else(|| "-".to_string());

        let regular_season: Vec<_> = league
            .schedule
            .iter()
            .filter(|game| game.date_index <= REGULAR_SEASON_DATES)
            .collect();
        Self {
            east,
            west,
            played: regular_season
                .iter()
                .filter(|game| game.status == GameStatus::Played)
                .count(),
            games: regular_season.len(),
            next_day,
        }
    }
}

#[derive(Template)]
#[template(path = "teams.html")]
struct TeamsTemplate {
    teams: Vec<TeamRow>,
}

impl TeamsTemplate {
    fn from_league(league: &League) -> Self {
        let mut teams: Vec<TeamRow> = league
            .teams
            .iter()
            .map(|team| TeamRow::from_team(league, team))
            .collect();
        teams.sort_by(|a, b| {
            a.conference
                .cmp(&b.conference)
                .then_with(|| a.division.cmp(&b.division))
                .then_with(|| a.city.cmp(&b.city))
        });
        Self { teams }
    }
}

#[derive(Template)]
#[template(path = "team.html")]
struct TeamTemplate {
    team: TeamRow,
    players: Vec<PlayerRow>,
    owned: bool,
    owner_name: String,
    signed_in: bool,
    viewer_owns: bool,
    can_claim: bool,
    can_propose: bool,
    lineup: Vec<LineupRow>,
}

struct LineupRow {
    id: String,
    name: String,
    position: String,
    overall: u8,
    starter: bool,
    minutes: String,
}

impl TeamTemplate {
    fn from_league(
        league: &League,
        users: &UserStore,
        viewer: Option<Uuid>,
        id: &str,
    ) -> Option<Self> {
        let team = league.teams.iter().find(|team| team.id == id)?;
        let season_stats = player_season_stats(league);
        let mut players: Vec<PlayerRow> = team
            .roster
            .iter()
            .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
            .map(|player| PlayerRow::from_player(player, season_stats.get(&player.id)))
            .collect();
        players.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
                .then_with(|| b.overall.cmp(&a.overall))
        });
        let owner_name = team
            .owner_user_id
            .and_then(|owner_id| users.get(owner_id))
            .map(|user| user.display_name)
            .unwrap_or_default();
        let viewer_team = viewer.and_then(|user_id| owned_team_id(league, user_id));
        let viewer_owns = viewer.is_some() && team.owner_user_id == viewer;
        let mut lineup: Vec<LineupRow> = team
            .roster
            .iter()
            .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
            .map(|player| LineupRow {
                id: player.id.clone(),
                name: player.name.clone(),
                position: position_name(player.position),
                overall: player_overall(player) as u8,
                starter: team.starters.contains(&player.id),
                minutes: team
                    .minute_targets
                    .get(&player.id)
                    .map(|minutes| minutes.to_string())
                    .unwrap_or_default(),
            })
            .collect();
        lineup.sort_by(|a, b| b.overall.cmp(&a.overall).then_with(|| a.id.cmp(&b.id)));
        Some(Self {
            team: TeamRow::from_team(league, team),
            players,
            owned: team.owner_user_id.is_some(),
            owner_name,
            signed_in: viewer.is_some(),
            viewer_owns,
            can_claim: viewer.is_some() && team.owner_user_id.is_none() && viewer_team.is_none(),
            can_propose: team.owner_user_id.is_some() && !viewer_owns && viewer_team.is_some(),
            lineup,
        })
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {}

#[derive(Template)]
#[template(path = "trades.html")]
struct TradesTemplate {
    signed_in: bool,
    has_team: bool,
    my_team: String,
    incoming: Vec<TradeRow>,
    outgoing: Vec<TradeRow>,
    history: Vec<TradeRow>,
    partners: Vec<TradePartner>,
}

struct TradePartner {
    id: String,
    label: String,
}

struct TradeRow {
    id: String,
    from_team: String,
    to_team: String,
    offered: String,
    requested: String,
    note: String,
    response_note: String,
    status: String,
}

impl TradeRow {
    fn from_trade(league: &League, trade: &crate::models::TradeOffer) -> Self {
        Self {
            id: trade.id.clone(),
            from_team: team_label(league, &trade.from_team_id),
            to_team: team_label(league, &trade.to_team_id),
            offered: player_names(league, &trade.offered_player_ids),
            requested: player_names(league, &trade.requested_player_ids),
            note: trade.note.clone().unwrap_or_default(),
            response_note: trade.response_note.clone().unwrap_or_default(),
            status: trade.status.to_string(),
        }
    }
}

impl TradesTemplate {
    fn from_league(league: &League, viewer: Option<Uuid>) -> Self {
        let my_team_id = viewer.and_then(|user_id| owned_team_id(league, user_id));
        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();
        let mut history = Vec::new();
        for trade in league.trades.iter().rev() {
            let row = TradeRow::from_trade(league, trade);
            let mine_in = my_team_id.as_deref() == Some(trade.to_team_id.as_str());
            let mine_out = my_team_id.as_deref() == Some(trade.from_team_id.as_str());
            if trade.status == TradeStatus::Pending && mine_in {
                incoming.push(row);
            } else if trade.status == TradeStatus::Pending && mine_out {
                outgoing.push(row);
            } else {
                history.push(row);
            }
        }
        let partners = league
            .teams
            .iter()
            .filter(|team| {
                team.owner_user_id.is_some() && Some(team.id.as_str()) != my_team_id.as_deref()
            })
            .map(|team| TradePartner {
                id: team.id.clone(),
                label: format!("{} {}", team.city, team.name),
            })
            .collect();
        Self {
            signed_in: viewer.is_some(),
            has_team: my_team_id.is_some(),
            my_team: my_team_id
                .as_deref()
                .map(|team_id| team_label(league, team_id))
                .unwrap_or_default(),
            incoming,
            outgoing,
            history,
            partners,
        }
    }
}

#[derive(Template)]
#[template(path = "trade_new.html")]
struct TradeNewTemplate {
    to_team_id: String,
    from_team: String,
    to_team: String,
    counter_of: String,
    my_players: Vec<TradePlayerRow>,
    their_players: Vec<TradePlayerRow>,
}

struct TradePlayerRow {
    id: String,
    name: String,
    position: String,
    overall: u8,
}

impl TradeNewTemplate {
    fn from_league(
        league: &League,
        from_team_id: &str,
        to_team_id: &str,
        counter_of: Option<String>,
    ) -> Option<Self> {
        let from = league.teams.iter().find(|team| team.id == from_team_id)?;
        let to = league.teams.iter().find(|team| team.id == to_team_id)?;
        let rows = |team: &Team| -> Vec<TradePlayerRow> {
            let mut rows: Vec<TradePlayerRow> = team
                .roster
                .iter()
                .filter_map(|player_id| {
                    league.players.iter().find(|player| &player.id == player_id)
                })
                .map(|player| TradePlayerRow {
                    id: player.id.clone(),
                    name: player.name.clone(),
                    position: position_name(player.position),
                    overall: player_overall(player) as u8,
                })
                .collect();
            rows.sort_by(|a, b| b.overall.cmp(&a.overall).then_with(|| a.id.cmp(&b.id)));
            rows
        };
        Some(Self {
            to_team_id: to.id.clone(),
            from_team: format!("{} {}", from.city, from.name),
            to_team: format!("{} {}", to.city, to.name),
            counter_of: counter_of.unwrap_or_default(),
            my_players: rows(from),
            their_players: rows(to),
        })
    }
}

#[derive(Template)]
#[template(path = "playoffs.html")]
struct PlayoffsTemplate {
    started: bool,
    can_start: bool,
    finished: bool,
    champion: String,
    rounds: Vec<PlayoffRoundView>,
}

struct PlayoffRoundView {
    name: String,
    series: Vec<PlayoffSeriesView>,
}

struct PlayoffSeriesView {
    label: String,
    high_team: String,
    low_team: String,
    high_wins: u8,
    low_wins: u8,
    finished: bool,
    winner: String,
    games: Vec<PlayoffGameView>,
}

struct PlayoffGameView {
    id: String,
    number: usize,
    score: String,
}

impl PlayoffsTemplate {
    fn from_league(league: &League) -> Self {
        let champion_label = champion(league)
            .map(|team_id| team_label(league, &team_id))
            .unwrap_or_default();
        let rounds = league
            .playoffs
            .as_ref()
            .map(|playoffs| {
                playoffs
                    .rounds
                    .iter()
                    .map(|round| PlayoffRoundView {
                        name: round.name.clone(),
                        series: round
                            .series
                            .iter()
                            .map(|series| PlayoffSeriesView {
                                label: match series.conference {
                                    Some(conference) => format!(
                                        "{} {} vs {}",
                                        conference, series.high_seed, series.low_seed
                                    ),
                                    None => "Finals".to_string(),
                                },
                                high_team: team_label(league, &series.high_team_id),
                                low_team: team_label(league, &series.low_team_id),
                                high_wins: series.high_wins,
                                low_wins: series.low_wins,
                                finished: series.finished(),
                                winner: series
                                    .winner_team_id
                                    .as_deref()
                                    .map(|team_id| team_label(league, team_id))
                                    .unwrap_or_default(),
                                games: series
                                    .game_ids
                                    .iter()
                                    .enumerate()
                                    .map(|(index, game_id)| PlayoffGameView {
                                        id: game_id.clone(),
                                        number: index + 1,
                                        score: league
                                            .results
                                            .get(game_id)
                                            .map(|result| {
                                                format!(
                                                    "{}-{}",
                                                    result.away_score, result.home_score
                                                )
                                            })
                                            .unwrap_or_else(|| "-".to_string()),
                                    })
                                    .collect(),
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            started: league.playoffs.is_some(),
            can_start: league.playoffs.is_none() && regular_season_complete(league),
            finished: !champion_label.is_empty(),
            champion: champion_label,
            rounds,
        }
    }
}

#[derive(Template)]
#[template(path = "player.html")]
struct PlayerTemplate {
    player: PlayerRow,
    team: TeamRow,
}

impl PlayerTemplate {
    fn from_league(league: &League, id: &str) -> Option<Self> {
        let player = league.players.iter().find(|player| player.id == id)?;
        let team = league.teams.iter().find(|team| team.id == player.team_id)?;
        let season_stats = player_season_stats(league);
        Some(Self {
            player: PlayerRow::from_player(player, season_stats.get(&player.id)),
            team: TeamRow::from_team(league, team),
        })
    }
}

#[derive(Template)]
#[template(path = "game.html")]
struct GameTemplate {
    game_id: String,
    date_index: u16,
    away: String,
    home: String,
    away_score: String,
    home_score: String,
    status: String,
    played: bool,
    teams: Vec<BoxScoreTeam>,
    has_pbp: bool,
    pbp_quarters: Vec<PbpQuarter>,
}

struct PbpQuarter {
    label: String,
    plays: Vec<PbpRow>,
}

struct PbpRow {
    clock: String,
    team: String,
    description: String,
    score: String,
}

impl GameTemplate {
    fn from_league(league: &League, id: &str) -> Option<Self> {
        let game = league.schedule.iter().find(|game| game.id == id)?;
        let home = league
            .teams
            .iter()
            .find(|team| team.id == game.home_team_id)?;
        let away = league
            .teams
            .iter()
            .find(|team| team.id == game.away_team_id)?;
        let result = league.results.get(&game.id);
        let player_lines = result
            .and_then(|result| result.player_stats.as_deref())
            .unwrap_or_default();
        let pie_total = pie_total(player_lines);
        let pbp_quarters = pbp_quarters(league, result);
        Some(Self {
            game_id: game.id.clone(),
            date_index: game.date_index,
            away: format!("{} {}", away.city, away.name),
            home: format!("{} {}", home.city, home.name),
            away_score: result
                .map(|result| result.away_score.to_string())
                .unwrap_or_else(|| "-".to_string()),
            home_score: result
                .map(|result| result.home_score.to_string())
                .unwrap_or_else(|| "-".to_string()),
            status: game_status(game, result),
            played: game.status == GameStatus::Played,
            teams: vec![
                box_score_team(
                    league,
                    away,
                    result.map(|result| result.away_score).unwrap_or_default(),
                    result.is_some_and(|result| result.winner_team_id == away.id),
                    player_lines,
                    pie_total,
                ),
                box_score_team(
                    league,
                    home,
                    result.map(|result| result.home_score).unwrap_or_default(),
                    result.is_some_and(|result| result.winner_team_id == home.id),
                    player_lines,
                    pie_total,
                ),
            ],
            has_pbp: !pbp_quarters.is_empty(),
            pbp_quarters,
        })
    }
}

fn pbp_quarters(league: &League, result: Option<&GameResult>) -> Vec<PbpQuarter> {
    let plays = result
        .and_then(|result| result.play_by_play.as_deref())
        .unwrap_or_default();
    let mut quarters: Vec<PbpQuarter> = Vec::new();
    for play in plays {
        let team = league
            .teams
            .iter()
            .find(|team| team.id == play.team_id)
            .map(|team| team.name.clone())
            .unwrap_or_else(|| play.team_id.clone());
        let label = quarter_label(play.quarter);
        if quarters.last().map(|quarter| quarter.label.as_str()) != Some(label) {
            quarters.push(PbpQuarter {
                label: label.to_string(),
                plays: Vec::new(),
            });
        }
        quarters.last_mut().expect("quarter").plays.push(PbpRow {
            clock: play.clock.clone(),
            team,
            description: play.description.clone(),
            score: format!("{} - {}", play.away_score, play.home_score),
        });
    }
    quarters
}

fn quarter_label(quarter: u8) -> &'static str {
    match quarter {
        1 => "1st Quarter",
        2 => "2nd Quarter",
        3 => "3rd Quarter",
        _ => "4th Quarter",
    }
}

#[derive(Template)]
#[template(path = "schedule.html")]
struct ScheduleTemplate {
    games: Vec<GameRow>,
}

impl ScheduleTemplate {
    fn from_league(league: &League) -> Self {
        let games = league
            .schedule
            .iter()
            .map(|game| {
                let home = league
                    .teams
                    .iter()
                    .find(|team| team.id == game.home_team_id)
                    .expect("home team");
                let away = league
                    .teams
                    .iter()
                    .find(|team| team.id == game.away_team_id)
                    .expect("away team");
                let result = league.results.get(&game.id);
                GameRow {
                    id: game.id.clone(),
                    date_index: game.date_index,
                    home: format!("{} {}", home.city, home.name),
                    away: format!("{} {}", away.city, away.name),
                    status: match game.status {
                        GameStatus::Scheduled => "Scheduled".to_string(),
                        GameStatus::Played => "Final".to_string(),
                    },
                    score: result
                        .map(|result| format!("{}-{}", result.away_score, result.home_score))
                        .unwrap_or_else(|| "-".to_string()),
                    played: game.status == GameStatus::Played,
                }
            })
            .collect();
        Self { games }
    }
}

#[derive(Clone)]
struct TeamRow {
    id: String,
    city: String,
    name: String,
    conference: String,
    division: String,
    roster: usize,
    rating: i16,
}

impl TeamRow {
    fn from_team(league: &League, team: &Team) -> Self {
        Self {
            id: team.id.clone(),
            city: team.city.clone(),
            name: team.name.clone(),
            conference: team.conference.to_string(),
            division: team.division.to_string(),
            roster: team.roster.len(),
            rating: team_rating(league, team),
        }
    }
}

#[derive(Clone)]
struct StandingRow {
    team_id: String,
    team: String,
    conference: String,
    wins: u16,
    losses: u16,
    pct: String,
    points_for: u16,
    points_against: u16,
    diff: i16,
}

impl StandingRow {
    fn from_team(league: &League, team: &Team, record: &crate::stats::TeamRecord) -> Self {
        let _ = league;
        Self {
            team_id: team.id.clone(),
            team: format!("{} {}", team.city, team.name),
            conference: team.conference.to_string(),
            wins: record.wins,
            losses: record.losses,
            pct: record.pct(),
            points_for: record.points_for,
            points_against: record.points_against,
            diff: record.differential(),
        }
    }
}

struct PlayerRow {
    id: String,
    name: String,
    position: String,
    age: u8,
    two_point_pct: u8,
    three_point_pct: u8,
    ft_pct: u8,
    inside_scoring: u8,
    three_tendency: u8,
    passing: u8,
    ball_handling: u8,
    perimeter_defense: u8,
    interior_defense: u8,
    steal: u8,
    block: u8,
    offensive_rebounding: u8,
    defensive_rebounding: u8,
    overall: u8,
    games: u16,
    minutes: u16,
    points: u16,
    rebounds: u16,
    assists: u16,
    steals: u16,
    blocks: u16,
    turnovers: u16,
    fouls: u16,
    fgm_fga: String,
    tpm_tpa: String,
    ftm_fta: String,
}

impl PlayerRow {
    fn from_player(player: &Player, stats: Option<&PlayerSeasonStats>) -> Self {
        let overall = player_overall(player) as u8;
        let empty = PlayerSeasonStats {
            player_id: player.id.clone(),
            ..PlayerSeasonStats::default()
        };
        let stats = stats.unwrap_or(&empty);
        Self {
            id: player.id.clone(),
            name: player.name.clone(),
            position: position_name(player.position),
            age: player.age,
            two_point_pct: player.ratings.two_point_pct,
            three_point_pct: player.ratings.three_point_pct,
            ft_pct: player.ratings.ft_pct,
            inside_scoring: player.ratings.inside_scoring,
            three_tendency: player.ratings.three_tendency,
            passing: player.ratings.passing,
            ball_handling: player.ratings.ball_handling,
            perimeter_defense: player.ratings.perimeter_defense,
            interior_defense: player.ratings.interior_defense,
            steal: player.ratings.steal,
            block: player.ratings.block,
            offensive_rebounding: player.ratings.offensive_rebounding,
            defensive_rebounding: player.ratings.defensive_rebounding,
            overall,
            games: stats.games,
            minutes: stats.minutes,
            points: stats.points,
            rebounds: stats.rebounds,
            assists: stats.assists,
            steals: stats.steals,
            blocks: stats.blocks,
            turnovers: stats.turnovers,
            fouls: stats.fouls,
            fgm_fga: made_attempted(stats.field_goals_made, stats.field_goals_attempted),
            tpm_tpa: made_attempted(stats.three_pointers_made, stats.three_pointers_attempted),
            ftm_fta: made_attempted(stats.free_throws_made, stats.free_throws_attempted),
        }
    }
}

struct GameRow {
    id: String,
    date_index: u16,
    home: String,
    away: String,
    status: String,
    score: String,
    played: bool,
}

struct BoxScoreRow {
    player_id: String,
    player: String,
    minutes: u16,
    points: u16,
    rebounds: u16,
    assists: u16,
    steals: u16,
    blocks: u16,
    turnovers: u16,
    fouls: u16,
    plus_minus: i16,
    fgm_fga: String,
    tpm_tpa: String,
    ftm_fta: String,
    pie: String,
    pps: String,
    usg: String,
}

struct BoxScoreTotals {
    minutes: u16,
    points: u16,
    rebounds: u16,
    assists: u16,
    fgm_fga: String,
    tpm_tpa: String,
    ftm_fta: String,
    pps: String,
    turnovers: u16,
    steals: u16,
    blocks: u16,
    fouls: u16,
}

#[derive(Default)]
struct NumericTeamTotals {
    minutes: u16,
    points: u16,
    rebounds: u16,
    assists: u16,
    field_goals_made: u16,
    field_goals_attempted: u16,
    three_pointers_made: u16,
    three_pointers_attempted: u16,
    free_throws_made: u16,
    free_throws_attempted: u16,
    turnovers: u16,
    steals: u16,
    blocks: u16,
    fouls: u16,
}

struct BoxScoreTeam {
    name: String,
    score: u16,
    winner: bool,
    rows: Vec<BoxScoreRow>,
    totals: BoxScoreTotals,
}

fn pie_total(lines: &[PlayerGameStats]) -> f64 {
    lines.iter().map(pie_component).sum()
}

fn pie_component(line: &PlayerGameStats) -> f64 {
    line.points as f64 + line.field_goals_made as f64 + line.free_throws_made as f64
        - line.field_goals_attempted as f64
        - line.free_throws_attempted as f64
        + line.rebounds as f64
        + line.assists as f64
        + line.steals as f64
        + line.blocks as f64 / 2.0
        - line.fouls as f64
        - line.turnovers as f64
}

fn box_score_team(
    league: &League,
    team: &Team,
    score: u16,
    winner: bool,
    lines: &[PlayerGameStats],
    game_pie_total: f64,
) -> BoxScoreTeam {
    let numeric_totals = lines.iter().filter(|line| line.team_id == team.id).fold(
        NumericTeamTotals::default(),
        |mut totals, line| {
            totals.minutes += line.minutes;
            totals.points += line.points;
            totals.rebounds += line.rebounds;
            totals.assists += line.assists;
            totals.field_goals_made += line.field_goals_made;
            totals.field_goals_attempted += line.field_goals_attempted;
            totals.three_pointers_made += line.three_pointers_made;
            totals.three_pointers_attempted += line.three_pointers_attempted;
            totals.free_throws_made += line.free_throws_made;
            totals.free_throws_attempted += line.free_throws_attempted;
            totals.turnovers += line.turnovers;
            totals.steals += line.steals;
            totals.blocks += line.blocks;
            totals.fouls += line.fouls;
            totals
        },
    );
    let mut rows: Vec<BoxScoreRow> = lines
        .iter()
        .filter(|line| line.team_id == team.id)
        .filter_map(|line| {
            let player = league
                .players
                .iter()
                .find(|player| player.id == line.player_id)?;
            Some(BoxScoreRow {
                player_id: player.id.clone(),
                player: player.name.clone(),
                minutes: line.minutes,
                points: line.points,
                rebounds: line.rebounds,
                assists: line.assists,
                steals: line.steals,
                blocks: line.blocks,
                turnovers: line.turnovers,
                fouls: line.fouls,
                plus_minus: line.plus_minus,
                fgm_fga: made_attempted(line.field_goals_made, line.field_goals_attempted),
                tpm_tpa: made_attempted(line.three_pointers_made, line.three_pointers_attempted),
                ftm_fta: made_attempted(line.free_throws_made, line.free_throws_attempted),
                pie: format_pie(pie_component(line), game_pie_total),
                pps: format_pps(line.points, line.field_goals_attempted),
                usg: format_usg(line, &numeric_totals),
            })
        })
        .collect();
    rows.sort_by_key(|row| Reverse(row.minutes));

    let totals = BoxScoreTotals {
        minutes: numeric_totals.minutes,
        points: numeric_totals.points,
        rebounds: numeric_totals.rebounds,
        assists: numeric_totals.assists,
        fgm_fga: made_attempted(
            numeric_totals.field_goals_made,
            numeric_totals.field_goals_attempted,
        ),
        tpm_tpa: made_attempted(
            numeric_totals.three_pointers_made,
            numeric_totals.three_pointers_attempted,
        ),
        ftm_fta: made_attempted(
            numeric_totals.free_throws_made,
            numeric_totals.free_throws_attempted,
        ),
        pps: format_pps(numeric_totals.points, numeric_totals.field_goals_attempted),
        turnovers: numeric_totals.turnovers,
        steals: numeric_totals.steals,
        blocks: numeric_totals.blocks,
        fouls: numeric_totals.fouls,
    };

    BoxScoreTeam {
        name: format!("{} {}", team.city, team.name),
        score,
        winner,
        rows,
        totals,
    }
}

fn format_pie(component: f64, total: f64) -> String {
    if total == 0.0 {
        "0".to_string()
    } else {
        (100.0 * component / total).round().to_string()
    }
}

fn format_pps(points: u16, field_goals_attempted: u16) -> String {
    if field_goals_attempted == 0 {
        String::new()
    } else {
        format!(
            "{:.2}",
            ((points as f64 * 100.0) / field_goals_attempted as f64).round() / 100.0
        )
    }
}

fn format_usg(line: &PlayerGameStats, team: &NumericTeamTotals) -> String {
    let minutes = line.minutes as f64;
    if minutes <= 0.0 {
        return String::new();
    }
    let numerator = (line.field_goals_attempted as f64
        + 0.44 * line.free_throws_attempted as f64
        + line.turnovers as f64)
        * (team.minutes as f64 / 5.0);
    let denominator = minutes
        * (team.field_goals_attempted as f64
            + 0.44 * team.free_throws_attempted as f64
            + team.turnovers as f64);
    if denominator == 0.0 {
        String::new()
    } else {
        (100.0 * numerator / denominator).round().to_string()
    }
}

fn game_status(game: &Game, result: Option<&GameResult>) -> String {
    match (game.status, result) {
        (GameStatus::Played, Some(_)) => "Final".to_string(),
        _ => "Scheduled".to_string(),
    }
}

fn made_attempted(made: u16, attempted: u16) -> String {
    format!("{made}-{attempted}")
}

fn position_name(position: Position) -> String {
    position.to_string()
}

#[allow(dead_code)]
fn _conference_name(conference: Conference) -> String {
    conference.to_string()
}
