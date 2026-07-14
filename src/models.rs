use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type TeamId = String;
pub type PlayerId = String;
pub type GameId = String;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct League {
    pub id: String,
    pub name: String,
    pub seed: u64,
    pub season: u16,
    pub teams: Vec<Team>,
    pub players: Vec<Player>,
    pub schedule: Vec<Game>,
    pub results: BTreeMap<GameId, GameResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Team {
    pub id: TeamId,
    pub city: String,
    pub name: String,
    pub conference: Conference,
    pub division: Division,
    pub roster: Vec<PlayerId>,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Conference {
    East,
    West,
}

impl std::fmt::Display for Conference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::East => write!(f, "East"),
            Self::West => write!(f, "West"),
        }
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Division {
    Atlantic,
    Central,
    Southeast,
    Northwest,
    Pacific,
    Southwest,
}

impl std::fmt::Display for Division {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Atlantic => "Atlantic",
            Self::Central => "Central",
            Self::Southeast => "Southeast",
            Self::Northwest => "Northwest",
            Self::Pacific => "Pacific",
            Self::Southwest => "Southwest",
        };
        write!(f, "{name}")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub age: u8,
    pub position: Position,
    pub ratings: Ratings,
    pub team_id: TeamId,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Position {
    PG,
    SG,
    SF,
    PF,
    C,
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::PG => "PG",
            Self::SG => "SG",
            Self::SF => "SF",
            Self::PF => "PF",
            Self::C => "C",
        };
        write!(f, "{name}")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Ratings {
    pub offense: u8,
    pub defense: u8,
    pub shooting: u8,
    pub playmaking: u8,
    pub rebounding: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Game {
    pub id: GameId,
    pub season: u16,
    pub date_index: u16,
    pub home_team_id: TeamId,
    pub away_team_id: TeamId,
    pub status: GameStatus,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GameStatus {
    Scheduled,
    Played,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GameResult {
    pub game_id: GameId,
    pub home_score: u16,
    pub away_score: u16,
    pub winner_team_id: TeamId,
    pub team_stats: Option<TeamStats>,
    pub player_stats: Option<Vec<PlayerGameStats>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamStats {
    pub possessions: u16,
    pub offensive_rating: u16,
    pub defensive_rating: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlayerGameStats {
    pub player_id: PlayerId,
    pub team_id: TeamId,
    #[serde(default)]
    pub plus_minus: i16,
    pub minutes: u16,
    pub points: u16,
    pub rebounds: u16,
    pub assists: u16,
    pub steals: u16,
    pub blocks: u16,
    pub turnovers: u16,
    pub fouls: u16,
    pub field_goals_attempted: u16,
    pub field_goals_made: u16,
    pub three_pointers_attempted: u16,
    pub three_pointers_made: u16,
    pub free_throws_attempted: u16,
    pub free_throws_made: u16,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlayerSeasonStats {
    pub player_id: PlayerId,
    pub games: u16,
    pub minutes: u16,
    pub points: u16,
    pub rebounds: u16,
    pub assists: u16,
    pub steals: u16,
    pub blocks: u16,
    pub turnovers: u16,
    pub fouls: u16,
    pub field_goals_attempted: u16,
    pub field_goals_made: u16,
    pub three_pointers_attempted: u16,
    pub three_pointers_made: u16,
    pub free_throws_attempted: u16,
    pub free_throws_made: u16,
}
