use crate::models::{Conference, GameStatus, League, Player, PlayerSeasonStats, Position, Team};
use crate::repo::LeagueRepository;
use crate::sim::{SimConfig, simulate_game, team_rating};
use crate::stats::{next_unplayed_date_indices, player_season_stats, standings};
use askama::Template;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct AppState {
    pub repo: LeagueRepository,
    pub league: Arc<Mutex<League>>,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/standings", get(standings_page))
        .route("/teams", get(teams))
        .route("/teams/{id}", get(team_detail))
        .route("/players/{id}", get(player_detail))
        .route("/schedule", get(schedule))
        .route("/games/{id}/simulate", post(simulate))
        .route("/sim/day", post(sim_day))
        .route("/sim/week", post(sim_week))
        .route("/sim/month", post(sim_month))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
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

async fn team_detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    match TeamTemplate::from_league(&league, &id) {
        Some(template) => render(template),
        None => (axum::http::StatusCode::NOT_FOUND, "team not found").into_response(),
    }
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

async fn simulate(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    persist_simulation(state, |league| {
        simulate_game(league, &id, SimConfig::default());
    })
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
        leaders.sort_by(|a, b| b.rating.cmp(&a.rating));
        leaders.truncate(8);

        Self {
            league_name: league.name.clone(),
            season: league.season,
            teams: league.teams.len(),
            players: league.players.len(),
            games: league.schedule.len(),
            played: league.results.len(),
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

        Self {
            east,
            west,
            played: league.results.len(),
            games: league.schedule.len(),
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
}

impl TeamTemplate {
    fn from_league(league: &League, id: &str) -> Option<Self> {
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
        Some(Self {
            team: TeamRow::from_team(league, team),
            players,
        })
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
    offense: u8,
    defense: u8,
    shooting: u8,
    playmaking: u8,
    rebounding: u8,
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
        let overall = ((player.ratings.offense as u16
            + player.ratings.defense as u16
            + player.ratings.shooting as u16
            + player.ratings.playmaking as u16
            + player.ratings.rebounding as u16)
            / 5) as u8;
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
            offense: player.ratings.offense,
            defense: player.ratings.defense,
            shooting: player.ratings.shooting,
            playmaking: player.ratings.playmaking,
            rebounding: player.ratings.rebounding,
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
