use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub type TeamId = String;
pub type PlayerId = String;
pub type GameId = String;
pub type TradeId = String;

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
    #[serde(default)]
    pub trades: Vec<TradeOffer>,
    #[serde(default)]
    pub playoffs: Option<Playoffs>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Team {
    pub id: TeamId,
    pub city: String,
    pub name: String,
    pub conference: Conference,
    pub division: Division,
    pub roster: Vec<PlayerId>,
    #[serde(default)]
    pub owner_user_id: Option<Uuid>,
    /// Player ids the owner has designated as starters (exactly 5 when set).
    #[serde(default)]
    pub starters: Vec<PlayerId>,
    /// Per-player minute targets (0-48) that steer the rotation.
    #[serde(default)]
    pub minute_targets: BTreeMap<PlayerId, u16>,
    /// Explicit lineup mode; `None` on old saves (see [`Team::mode`]).
    #[serde(default)]
    pub lineup_mode: Option<LineupMode>,
    /// Bench priority order (best first) after the starters; used by the
    /// lineup editor and as the tie-break when the coach needs a body.
    #[serde(default)]
    pub bench_order: Vec<PlayerId>,
    /// `Chart` mode: twelve 4-minute blocks, each listing the five players
    /// on the floor.
    #[serde(default)]
    pub chart: Vec<Vec<PlayerId>>,
    #[serde(default)]
    pub strategy: TeamStrategy,
    #[serde(default)]
    pub coach: CoachSettings,
}

pub const CHART_BLOCKS: usize = 12;
pub const CHART_BLOCK_MINUTES: u16 = 4;

/// How a team's rotation is decided.
#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum LineupMode {
    /// The coach AI builds the rotation from roster quality and the coach sliders.
    #[default]
    Auto,
    /// Owner picks starters and per-player minute targets.
    Minutes,
    /// Owner paints exactly who is on the floor in each 4-minute block.
    Chart,
}

impl Team {
    /// Effective lineup mode: old saves with custom starters behave as `Minutes`.
    pub fn mode(&self) -> LineupMode {
        match self.lineup_mode {
            Some(mode) => mode,
            None if self.starters.len() == 5 => LineupMode::Minutes,
            None => LineupMode::Auto,
        }
    }
}

/// Team-wide play style. 0-100 per slider, 50 is neutral.
#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TeamStrategy {
    /// Offense: faster = more possessions and more transition, more fatigue.
    pub pace: u8,
    /// Offense: share of shots taken from three.
    pub three_rate: u8,
    /// Offense: low = isolation/star-heavy usage, high = share the ball.
    pub ball_movement: u8,
    /// Offense: crash the offensive glass (leaves you open to fast breaks).
    pub offensive_glass: u8,
    /// Defense: gamble for steals (more steals + fouls + fatigue).
    pub pressure: u8,
    /// Defense: low = guard the arc, high = pack the paint.
    pub interior_focus: u8,
    /// Defense: crash the defensive glass (fewer players leak out on offense).
    pub defensive_glass: u8,
}

impl Default for TeamStrategy {
    fn default() -> Self {
        Self {
            pace: 50,
            three_rate: 50,
            ball_movement: 50,
            offensive_glass: 50,
            pressure: 50,
            interior_focus: 50,
            defensive_glass: 50,
        }
    }
}

impl TeamStrategy {
    pub fn clamped(mut self) -> Self {
        for value in [
            &mut self.pace,
            &mut self.three_rate,
            &mut self.ball_movement,
            &mut self.offensive_glass,
            &mut self.pressure,
            &mut self.interior_focus,
            &mut self.defensive_glass,
        ] {
            *value = (*value).min(100);
        }
        self
    }
}

/// Substitution philosophy shared by every lineup mode.
#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct CoachSettings {
    /// Pull players in foul trouble early (high) or let them play (low).
    pub foul_caution: u8,
    /// Lean on the starters (high = ~38 min) or spread minutes (low = ~28).
    pub starter_load: u8,
    /// Players in the rotation, 7-12 (Auto mode).
    pub depth: u8,
    /// Play through tiredness (high) or sub at the first sign of fatigue (low).
    pub fatigue_tolerance: u8,
    /// Keep the best five on the floor in close games late.
    pub closers: bool,
    /// Rest starters in blowouts.
    pub blowout_bench: bool,
    /// Keep at least one of the two best players on the floor (Auto mode).
    pub stagger_stars: bool,
}

impl Default for CoachSettings {
    fn default() -> Self {
        Self {
            foul_caution: 50,
            starter_load: 50,
            depth: 9,
            fatigue_tolerance: 50,
            closers: true,
            blowout_bench: true,
            stagger_stars: true,
        }
    }
}

impl CoachSettings {
    pub fn clamped(mut self) -> Self {
        self.foul_caution = self.foul_caution.min(100);
        self.starter_load = self.starter_load.min(100);
        self.fatigue_tolerance = self.fatigue_tolerance.min(100);
        self.depth = self.depth.clamp(7, 12);
        self
    }
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
    pub two_point_pct: u8,
    pub three_point_pct: u8,
    pub ft_pct: u8,
    pub inside_scoring: u8,
    pub three_tendency: u8,
    pub passing: u8,
    pub ball_handling: u8,
    pub perimeter_defense: u8,
    pub interior_defense: u8,
    pub steal: u8,
    pub block: u8,
    pub offensive_rebounding: u8,
    pub defensive_rebounding: u8,
    /// Stamina: how slowly a player tires on the floor.
    #[serde(default = "default_endurance")]
    pub endurance: u8,
}

fn default_endurance() -> u8 {
    60
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
pub struct TradeOffer {
    pub id: TradeId,
    pub from_team_id: TeamId,
    pub to_team_id: TeamId,
    pub offered_player_ids: Vec<PlayerId>,
    pub requested_player_ids: Vec<PlayerId>,
    #[serde(default)]
    pub note: Option<String>,
    pub status: TradeStatus,
    /// Note attached by the receiving owner when rejecting or countering.
    #[serde(default)]
    pub response_note: Option<String>,
    /// When this offer is a counter, the id of the offer it replaces.
    #[serde(default)]
    pub counter_of: Option<TradeId>,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TradeStatus {
    Pending,
    Accepted,
    Rejected,
    Withdrawn,
    Countered,
}

impl std::fmt::Display for TradeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Pending => "Pending",
            Self::Accepted => "Accepted",
            Self::Rejected => "Rejected",
            Self::Withdrawn => "Withdrawn",
            Self::Countered => "Countered",
        };
        write!(f, "{name}")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Playoffs {
    pub season: u16,
    pub rounds: Vec<PlayoffRound>,
    /// Next playoff date index (continues after the regular season).
    pub next_date_index: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlayoffRound {
    pub number: u8,
    pub name: String,
    pub series: Vec<PlayoffSeries>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlayoffSeries {
    pub id: String,
    pub conference: Option<Conference>,
    pub high_seed: u8,
    pub low_seed: u8,
    pub high_team_id: TeamId,
    pub low_team_id: TeamId,
    pub game_ids: Vec<GameId>,
    pub high_wins: u8,
    pub low_wins: u8,
    pub winner_team_id: Option<TeamId>,
}

impl PlayoffSeries {
    pub fn finished(&self) -> bool {
        self.winner_team_id.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GameResult {
    pub game_id: GameId,
    pub home_score: u16,
    pub away_score: u16,
    pub winner_team_id: TeamId,
    pub team_stats: Option<TeamStats>,
    pub player_stats: Option<Vec<PlayerGameStats>>,
    #[serde(default)]
    pub play_by_play: Option<Vec<PlayEvent>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlayEvent {
    pub quarter: u8,
    pub clock: String,
    pub team_id: TeamId,
    pub description: String,
    pub away_score: u16,
    pub home_score: u16,
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
