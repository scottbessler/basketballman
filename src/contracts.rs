//! Salaries, the hard cap and roster transactions.
//!
//! Money is in thousands of dollars per year. The cap is *hard*: no signing
//! (free agent or re-sign) may push a payroll above [`SALARY_CAP`], and trades
//! must leave both sides under it. The only exceptions are draft picks
//! (rookie scale) and minimum contracts that fill a roster below
//! [`ROSTER_MIN`]; [`crate::offseason::enforce_roster_rules`] cleans up at the
//! start of a season.

use crate::config::{MAX_SALARY, MIN_SALARY, ROSTER_MAX, ROSTER_MIN, SALARY_CAP};
use crate::models::{Contract, League, Player, PlayerId, PlayerStatus, TeamId};
use crate::sim::player_overall;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ContractError {
    #[error("team not found")]
    NoTeam,
    #[error("player not found")]
    NoPlayer,
    #[error("player is not a free agent")]
    NotFreeAgent,
    #[error("roster is full ({0} players)")]
    RosterFull(usize),
    #[error("not enough cap room: need ${need}M, have ${have}M", need = fmt_m(*.0), have = fmt_m(*.1))]
    OverCap(u32, u32),
    #[error("player is not on that team")]
    WrongTeam,
    #[error("only expiring contracts can be re-signed, during the re-signing phase")]
    NotExpiring,
    #[error("contracts run 1 to 5 years")]
    BadYears,
    #[error("a team needs at least 5 players")]
    TooFewPlayers,
}

pub fn fmt_m(thousands: u32) -> String {
    format!("{:.1}", thousands as f64 / 1000.0)
}

/// Dollars-and-cents label like "$12.3M".
pub fn money(thousands: u32) -> String {
    format!("${}M", fmt_m(thousands))
}

/// What the league would pay a player of this quality. Stars are worth far
/// more than role players: salary grows roughly with the square of how far
/// the player is above a replacement-level 58 overall. Young high-potential
/// players are paid partly for what they will become.
pub fn market_salary(player: &Player) -> u32 {
    let overall = player_overall(player) as f64;
    let potential = (player.potential as f64).max(overall);
    let upside = if player.age <= 24 {
        (potential - overall) * 0.3
    } else {
        0.0
    };
    let value = overall + upside;
    let fraction = ((value - 60.0) / 34.0).clamp(0.0, 1.0).powf(2.2);
    let salary = MIN_SALARY as f64 + (MAX_SALARY - MIN_SALARY) as f64 * fraction;
    round_salary(salary as u32)
}

fn round_salary(salary: u32) -> u32 {
    ((salary + 50) / 100 * 100).clamp(MIN_SALARY, MAX_SALARY)
}

/// A free agent's current asking salary. Asks fall ~1.2% per day of free
/// agency down to 60% of market value.
pub fn ask_salary(player: &Player, fa_day: u16) -> u32 {
    let decay = (1.0 - 0.012 * fa_day as f64).max(0.6);
    round_salary((market_salary(player) as f64 * decay) as u32)
}

/// Contract length a player of this age prefers (older = shorter).
pub fn preferred_years(player: &Player) -> u8 {
    match player.age {
        0..=24 => 4,
        25..=28 => 4,
        29..=31 => 3,
        32..=33 => 2,
        _ => 1,
    }
}

/// First-round picks sign 3-year scale deals, second-rounders 2 years.
pub fn rookie_contract(pick_number: u16) -> Contract {
    let salary = round_salary((12_500.0 * 0.93f64.powi(pick_number as i32 - 1)) as u32);
    Contract {
        salary,
        years_left: if pick_number <= 30 { 3 } else { 2 },
    }
}

/// Committed salary for next season: expiring (0-year) deals do not count.
pub fn payroll(league: &League, team_id: &str) -> u32 {
    league
        .teams
        .iter()
        .find(|team| team.id == team_id)
        .map(|team| {
            team.roster
                .iter()
                .filter_map(|id| league.players.iter().find(|player| &player.id == id))
                .filter_map(|player| player.contract)
                .filter(|contract| contract.years_left > 0)
                .map(|contract| contract.salary)
                .sum()
        })
        .unwrap_or(0)
}

pub fn cap_room(league: &League, team_id: &str) -> u32 {
    SALARY_CAP.saturating_sub(payroll(league, team_id))
}

fn team_index(league: &League, team_id: &str) -> Result<usize, ContractError> {
    league
        .teams
        .iter()
        .position(|team| team.id == team_id)
        .ok_or(ContractError::NoTeam)
}

fn player_index(league: &League, player_id: &str) -> Result<usize, ContractError> {
    league
        .players
        .iter()
        .position(|player| player.id == player_id)
        .ok_or(ContractError::NoPlayer)
}

/// Sign a free agent at his current asking price.
pub fn sign_free_agent(
    league: &mut League,
    team_id: &str,
    player_id: &str,
    years: u8,
) -> Result<Contract, ContractError> {
    if !(1..=5).contains(&years) {
        return Err(ContractError::BadYears);
    }
    let team_i = team_index(league, team_id)?;
    let player_i = player_index(league, player_id)?;
    let player = &league.players[player_i];
    if player.status != PlayerStatus::FreeAgent {
        return Err(ContractError::NotFreeAgent);
    }
    let roster_len = league.teams[team_i].roster.len();
    if roster_len >= ROSTER_MAX {
        return Err(ContractError::RosterFull(roster_len));
    }
    let salary = ask_salary(player, league.fa_day);
    let room = cap_room(league, team_id);
    let min_exception = salary == MIN_SALARY && roster_len < ROSTER_MIN;
    if salary > room && !min_exception {
        return Err(ContractError::OverCap(salary, room));
    }
    let contract = Contract {
        salary,
        years_left: years,
    };
    let player = &mut league.players[player_i];
    player.contract = Some(contract);
    player.status = PlayerStatus::Active;
    player.team_id = team_id.to_string();
    let name = player.name.clone();
    league.teams[team_i].roster.push(player_id.to_string());
    let label = team_label(league, team_id);
    league.transactions.push(format!(
        "{label} signed {name} ({}, {years}y)",
        money(contract.salary)
    ));
    Ok(contract)
}

/// Extend an expiring player at his market price.
pub fn resign_player(
    league: &mut League,
    team_id: &str,
    player_id: &str,
    years: u8,
) -> Result<Contract, ContractError> {
    if !(1..=5).contains(&years) {
        return Err(ContractError::BadYears);
    }
    let team_i = team_index(league, team_id)?;
    if !league.teams[team_i].roster.iter().any(|id| id == player_id) {
        return Err(ContractError::WrongTeam);
    }
    let player_i = player_index(league, player_id)?;
    let player = &league.players[player_i];
    let expiring = player.contract.is_some_and(|c| c.years_left == 0);
    if !expiring || league.phase != crate::models::Phase::Resign {
        return Err(ContractError::NotExpiring);
    }
    let salary = market_salary(player);
    let room = cap_room(league, team_id);
    if salary > room {
        return Err(ContractError::OverCap(salary, room));
    }
    let contract = Contract {
        salary,
        years_left: years,
    };
    let name = player.name.clone();
    league.players[player_i].contract = Some(contract);
    let label = team_label(league, team_id);
    league.transactions.push(format!(
        "{label} re-signed {name} ({}, {years}y)",
        money(salary)
    ));
    Ok(contract)
}

/// Release a player to free agency; the remaining contract is forgiven.
pub fn waive_player(
    league: &mut League,
    team_id: &str,
    player_id: &str,
) -> Result<(), ContractError> {
    let team_i = team_index(league, team_id)?;
    if !league.teams[team_i].roster.iter().any(|id| id == player_id) {
        return Err(ContractError::WrongTeam);
    }
    if league.teams[team_i].roster.len() <= 5 {
        return Err(ContractError::TooFewPlayers);
    }
    let player_i = player_index(league, player_id)?;
    release_to_pool(league, team_i, player_i, PlayerStatus::FreeAgent);
    let name = league.players[player_i].name.clone();
    let label = team_label(league, team_id);
    league.transactions.push(format!("{label} waived {name}"));
    Ok(())
}

/// Remove a player from his team's roster and lineup settings.
pub fn release_to_pool(league: &mut League, team_i: usize, player_i: usize, status: PlayerStatus) {
    let player_id = league.players[player_i].id.clone();
    let team = &mut league.teams[team_i];
    team.roster.retain(|id| id != &player_id);
    team.forget_player(&player_id);
    let player = &mut league.players[player_i];
    player.team_id = String::new();
    player.status = status;
    player.contract = None;
}

pub fn team_label(league: &League, team_id: &str) -> String {
    league
        .teams
        .iter()
        .find(|team| team.id == team_id)
        .map(|team| format!("{} {}", team.city, team.name))
        .unwrap_or_else(|| team_id.to_string())
}

/// Free agents best first (by overall, then id).
pub fn free_agents(league: &League) -> Vec<&Player> {
    let mut agents: Vec<&Player> = league
        .players
        .iter()
        .filter(|player| player.status == PlayerStatus::FreeAgent)
        .collect();
    agents.sort_by_key(|player| (std::cmp::Reverse(player_overall(player)), player.id.clone()));
    agents
}

/// Roster players ordered by on-court value (for AI cuts).
pub fn roster_by_value<'a>(league: &'a League, team_id: &str) -> Vec<&'a Player> {
    let Some(team) = league.teams.iter().find(|team| team.id == team_id) else {
        return Vec::new();
    };
    let mut players: Vec<&Player> = team
        .roster
        .iter()
        .filter_map(|id| league.players.iter().find(|player| &player.id == id))
        .collect();
    players.sort_by_key(|player| (std::cmp::Reverse(player_overall(player)), player.id.clone()));
    players
}

pub fn contract_of(league: &League, player_id: &PlayerId) -> Option<Contract> {
    league
        .players
        .iter()
        .find(|player| &player.id == player_id)
        .and_then(|player| player.contract)
}

/// Trade check: after swapping, a team must be under the cap or not have
/// taken on salary.
pub fn trade_cap_ok(
    league: &League,
    team_id: &TeamId,
    incoming: &[PlayerId],
    outgoing: &[PlayerId],
) -> bool {
    let sum = |ids: &[PlayerId]| -> u32 {
        ids.iter()
            .filter_map(|id| contract_of(league, id))
            .filter(|contract| contract.years_left > 0)
            .map(|contract| contract.salary)
            .sum()
    };
    let (into, out) = (sum(incoming), sum(outgoing));
    let after = (payroll(league, team_id) + into).saturating_sub(out);
    after <= SALARY_CAP || into <= out
}
