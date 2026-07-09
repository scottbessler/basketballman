use crate::models::{GameStatus, League, Player, Team};
use crate::repo::LeagueRepository;
use crate::sim::{SimConfig, simulate_game, team_rating};
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
        .route("/teams", get(teams))
        .route("/teams/{id}", get(team_detail))
        .route("/schedule", get(schedule))
        .route("/games/{id}/simulate", post(simulate))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
}

async fn index(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(IndexTemplate::from_league(&league))
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

async fn schedule(State(state): State<AppState>) -> Response {
    let league = state.league.lock().expect("league lock").clone();
    render(ScheduleTemplate::from_league(&league))
}

async fn simulate(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut league = state.league.lock().expect("league lock");
    if simulate_game(&mut league, &id, SimConfig::default()).is_some() {
        if let Err(error) = state.repo.save(&league) {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("save failed: {error}"),
            )
                .into_response();
        }
    }
    Redirect::to("/schedule").into_response()
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
        leaders.truncate(6);

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
        let mut players: Vec<PlayerRow> = team
            .roster
            .iter()
            .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
            .map(PlayerRow::from_player)
            .collect();
        players.sort_by(|a, b| b.overall.cmp(&a.overall));
        Some(Self {
            team: TeamRow::from_team(league, team),
            players,
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
                        GameStatus::Played => "Played".to_string(),
                    },
                    score: result
                        .map(|result| format!("{}-{}", result.home_score, result.away_score))
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

struct PlayerRow {
    name: String,
    position: String,
    age: u8,
    offense: u8,
    defense: u8,
    shooting: u8,
    playmaking: u8,
    rebounding: u8,
    overall: u8,
}

impl PlayerRow {
    fn from_player(player: &Player) -> Self {
        let overall = ((player.ratings.offense as u16
            + player.ratings.defense as u16
            + player.ratings.shooting as u16
            + player.ratings.playmaking as u16
            + player.ratings.rebounding as u16)
            / 5) as u8;
        Self {
            name: player.name.clone(),
            position: player.position.to_string(),
            age: player.age,
            offense: player.ratings.offense,
            defense: player.ratings.defense,
            shooting: player.ratings.shooting,
            playmaking: player.ratings.playmaking,
            rebounding: player.ratings.rebounding,
            overall,
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
