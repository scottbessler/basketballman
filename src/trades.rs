//! Player-for-player trade offers between team owners.

use crate::models::{League, PlayerId, TeamId, TradeOffer, TradeStatus};

pub const MAX_TRADE_NOTE: usize = 280;

/// Validation error shown to the proposing owner.
#[derive(Debug, thiserror::Error)]
pub enum TradeError {
    #[error("trade not found")]
    NotFound,
    #[error("trade is no longer pending")]
    NotPending,
    #[error("select at least one player on each side")]
    EmptySide,
    #[error("players must belong to the offering and receiving teams")]
    WrongTeam,
    #[error("a team cannot trade with itself")]
    SameTeam,
    #[error("both teams must be managed by a human owner")]
    Unowned,
}

pub fn next_trade_id(league: &League) -> String {
    format!("tr{:04}", league.trades.len() + 1)
}

pub fn validate_offer(
    league: &League,
    from_team_id: &TeamId,
    to_team_id: &TeamId,
    offered: &[PlayerId],
    requested: &[PlayerId],
) -> Result<(), TradeError> {
    if from_team_id == to_team_id {
        return Err(TradeError::SameTeam);
    }
    let from = team(league, from_team_id).ok_or(TradeError::NotFound)?;
    let to = team(league, to_team_id).ok_or(TradeError::NotFound)?;
    if from.owner_user_id.is_none() || to.owner_user_id.is_none() {
        return Err(TradeError::Unowned);
    }
    if offered.is_empty() || requested.is_empty() {
        return Err(TradeError::EmptySide);
    }
    if !offered.iter().all(|id| from.roster.contains(id))
        || !requested.iter().all(|id| to.roster.contains(id))
    {
        return Err(TradeError::WrongTeam);
    }
    Ok(())
}

/// Accept a pending offer: swap the players between the two rosters and clear
/// any lineup settings that referenced them.
pub fn accept_trade(league: &mut League, trade_id: &str) -> Result<(), TradeError> {
    let trade = league
        .trades
        .iter()
        .find(|trade| trade.id == trade_id)
        .cloned()
        .ok_or(TradeError::NotFound)?;
    if trade.status != TradeStatus::Pending {
        return Err(TradeError::NotPending);
    }
    // Re-validate: rosters may have changed since the offer was made.
    validate_offer(
        league,
        &trade.from_team_id,
        &trade.to_team_id,
        &trade.offered_player_ids,
        &trade.requested_player_ids,
    )?;

    move_players(
        league,
        &trade.offered_player_ids,
        &trade.from_team_id,
        &trade.to_team_id,
    );
    move_players(
        league,
        &trade.requested_player_ids,
        &trade.to_team_id,
        &trade.from_team_id,
    );

    let stored = league
        .trades
        .iter_mut()
        .find(|stored| stored.id == trade_id)
        .expect("trade");
    stored.status = TradeStatus::Accepted;
    Ok(())
}

fn move_players(league: &mut League, player_ids: &[PlayerId], from: &TeamId, to: &TeamId) {
    for player_id in player_ids {
        if let Some(team) = league.teams.iter_mut().find(|team| &team.id == from) {
            team.roster.retain(|id| id != player_id);
            team.starters.retain(|id| id != player_id);
            team.minute_targets.remove(player_id);
        }
        if let Some(team) = league.teams.iter_mut().find(|team| &team.id == to) {
            team.roster.push(player_id.clone());
        }
        if let Some(player) = league
            .players
            .iter_mut()
            .find(|player| &player.id == player_id)
        {
            player.team_id = to.clone();
        }
    }
}

pub fn trade_mut<'a>(league: &'a mut League, trade_id: &str) -> Option<&'a mut TradeOffer> {
    league.trades.iter_mut().find(|trade| trade.id == trade_id)
}

fn team<'a>(league: &'a League, team_id: &TeamId) -> Option<&'a crate::models::Team> {
    league.teams.iter().find(|team| &team.id == team_id)
}

/// Trim a note field to its length bound, returning `None` when empty.
pub fn clean_note(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(MAX_TRADE_NOTE).collect())
    }
}
