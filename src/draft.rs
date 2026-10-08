//! Drafts: the rookie draft after each season and the fantasy draft that can
//! build a brand-new league. AI teams pick by value with per-team taste noise;
//! human owners pick when on the clock.

use crate::config::{DRAFT_ROUNDS, FANTASY_ROUNDS, MIN_SALARY, ROSTER_MAX, SALARY_CAP};
use crate::contracts::{
    market_salary, money, payroll, preferred_years, rookie_contract, team_label,
};
use crate::models::{
    Contract, Draft, DraftKind, DraftPick, League, Phase, Player, PlayerStatus, Position, TeamId,
};
use crate::playoffs::conference_seeds;
use crate::sim::player_overall;
use crate::stats::standings;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum DraftError {
    #[error("there is no draft in progress")]
    NoDraft,
    #[error("the draft is over")]
    Complete,
    #[error("that team is not on the clock")]
    NotOnClock,
    #[error("that player is not available")]
    NotAvailable,
    #[error("picking him would leave no cap room to fill your roster")]
    OverCap,
    #[error("roster is full")]
    RosterFull,
}

/// Overall as scouts see it: prospects carry a fixed scouting error.
pub fn scouted_overall(player: &Player) -> u16 {
    let overall = player_overall(player) as i32;
    if player.status == PlayerStatus::Prospect {
        (overall + player.scouting_fudge as i32).clamp(30, 99) as u16
    } else {
        overall as u16
    }
}

pub fn scouted_potential(player: &Player) -> u16 {
    let potential = (player.potential as u16).max(player_overall(player));
    if player.status == PlayerStatus::Prospect {
        (potential as i32 + player.scouting_fudge as i32).clamp(30, 99) as u16
    } else {
        potential
    }
}

fn is_available(draft: &Draft, player: &Player) -> bool {
    match draft.kind {
        DraftKind::Fantasy => player.status == PlayerStatus::FreeAgent,
        DraftKind::Rookie => {
            player.status == PlayerStatus::Prospect && player.draft_year == Some(draft.season)
        }
    }
}

/// Players still on the board, best first by scouting value.
pub fn available_players(league: &League) -> Vec<&Player> {
    let Some(draft) = &league.draft else {
        return Vec::new();
    };
    let mut pool: Vec<&Player> = league
        .players
        .iter()
        .filter(|player| is_available(draft, player))
        .collect();
    pool.sort_by(|a, b| {
        board_value(draft.kind, b)
            .partial_cmp(&board_value(draft.kind, a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    pool
}

fn board_value(kind: DraftKind, player: &Player) -> f64 {
    match kind {
        DraftKind::Fantasy => player_overall(player) as f64,
        DraftKind::Rookie => {
            0.45 * scouted_overall(player) as f64 + 0.55 * scouted_potential(player) as f64
        }
    }
}

/// Salary a drafted player signs for. Rookie scale for the rookie draft,
/// market value for the fantasy draft.
pub fn draft_contract(kind: DraftKind, player: &Player, pick_number: u16) -> Contract {
    match kind {
        DraftKind::Rookie => rookie_contract(pick_number),
        DraftKind::Fantasy => {
            let wobble = (stable_hash(&player.id) % 3) as i32 - 1;
            Contract {
                salary: market_salary(player),
                years_left: (preferred_years(player) as i32 + wobble).clamp(1, 5) as u8,
            }
        }
    }
}

fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3)
    })
}

fn rng_for(league: &League, salt: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(league.seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Snake order over `rounds` with teams shuffled for round one.
pub fn fantasy_order(league: &League, rounds: usize) -> Vec<TeamId> {
    let mut rng = rng_for(league, 0xF4A7);
    let mut teams: Vec<TeamId> = league.teams.iter().map(|team| team.id.clone()).collect();
    for i in (1..teams.len()).rev() {
        teams.swap(i, rng.gen_range(0..=i));
    }
    let mut order = Vec::with_capacity(rounds * teams.len());
    for round in 0..rounds {
        if round % 2 == 0 {
            order.extend(teams.iter().cloned());
        } else {
            order.extend(teams.iter().rev().cloned());
        }
    }
    order
}

/// Two rounds: lottery for the 16 non-playoff teams' top four picks, then
/// worst record to best; round two is straight reverse record order.
pub fn rookie_order(league: &League) -> Vec<TeamId> {
    let records = standings(league);
    let mut playoff: Vec<TeamId> = Vec::new();
    for conference in [
        crate::models::Conference::East,
        crate::models::Conference::West,
    ] {
        playoff.extend(conference_seeds(league, conference));
    }
    let mut by_record: Vec<&TeamId> = league.teams.iter().map(|team| &team.id).collect();
    by_record.sort_by_key(|id| {
        let record = &records[*id];
        (record.wins, record.differential(), (*id).clone())
    });
    let lottery_teams: Vec<TeamId> = by_record
        .iter()
        .filter(|id| !playoff.contains(id))
        .map(|id| (*id).clone())
        .collect();
    let playoff_order: Vec<TeamId> = by_record
        .iter()
        .filter(|id| playoff.contains(id))
        .map(|id| (*id).clone())
        .collect();

    // Weighted draw (without replacement) for the top four, worst record first.
    const WEIGHTS: [f64; 16] = [
        14.0, 14.0, 14.0, 12.5, 10.5, 9.0, 7.5, 6.0, 4.5, 3.0, 2.0, 1.5, 1.0, 0.5, 0.25, 0.25,
    ];
    let mut rng = rng_for(league, 0x10_77 ^ league.season as u64);
    let mut remaining: Vec<(TeamId, f64)> = lottery_teams
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), WEIGHTS[i.min(WEIGHTS.len() - 1)]))
        .collect();
    let mut round_one: Vec<TeamId> = Vec::new();
    for _ in 0..4.min(remaining.len()) {
        let total: f64 = remaining.iter().map(|(_, w)| w).sum();
        let mut ticket = rng.gen_range(0.0..total);
        let mut chosen = remaining.len() - 1;
        for (i, (_, weight)) in remaining.iter().enumerate() {
            if ticket < *weight {
                chosen = i;
                break;
            }
            ticket -= weight;
        }
        round_one.push(remaining.remove(chosen).0);
    }
    round_one.extend(remaining.into_iter().map(|(id, _)| id));
    round_one.extend(playoff_order);

    let mut order = round_one;
    for _ in 1..DRAFT_ROUNDS {
        order.extend(by_record.iter().map(|id| (*id).clone()));
    }
    order
}

pub fn start_fantasy_draft(league: &mut League) {
    league.phase = Phase::FantasyDraft;
    league.draft = Some(Draft {
        kind: DraftKind::Fantasy,
        season: league.season,
        order: fantasy_order(league, FANTASY_ROUNDS),
        picks: Vec::new(),
    });
}

pub fn start_rookie_draft(league: &mut League) {
    league.phase = Phase::Draft;
    league.draft = Some(Draft {
        kind: DraftKind::Rookie,
        season: league.season,
        order: rookie_order(league),
        picks: Vec::new(),
    });
}

fn would_overspend(league: &League, team_id: &str, salary: u32) -> bool {
    let roster = league
        .teams
        .iter()
        .find(|team| team.id == team_id)
        .map(|team| team.roster.len())
        .unwrap_or(0);
    // After this pick, the remaining slots must still be fillable at the minimum.
    let still_needed = FANTASY_ROUNDS.saturating_sub(roster + 1) as u32;
    payroll(league, team_id) + salary + still_needed * MIN_SALARY > SALARY_CAP
}

/// Make a pick for the team on the clock.
pub fn make_pick(
    league: &mut League,
    team_id: &str,
    player_id: &str,
) -> Result<DraftPick, DraftError> {
    let draft = league.draft.as_ref().ok_or(DraftError::NoDraft)?;
    if draft.complete() {
        return Err(DraftError::Complete);
    }
    if draft.on_the_clock().map(String::as_str) != Some(team_id) {
        return Err(DraftError::NotOnClock);
    }
    let kind = draft.kind;
    let number = draft.picks.len() as u16 + 1;
    let player_index = league
        .players
        .iter()
        .position(|player| player.id == player_id)
        .ok_or(DraftError::NotAvailable)?;
    if !is_available(draft, &league.players[player_index]) {
        return Err(DraftError::NotAvailable);
    }
    let team_index = league
        .teams
        .iter()
        .position(|team| team.id == team_id)
        .ok_or(DraftError::NotOnClock)?;
    if league.teams[team_index].roster.len() >= ROSTER_MAX + 2 {
        return Err(DraftError::RosterFull);
    }
    let contract = draft_contract(kind, &league.players[player_index], number);
    if kind == DraftKind::Fantasy && would_overspend(league, team_id, contract.salary) {
        return Err(DraftError::OverCap);
    }

    let season = league
        .draft
        .as_ref()
        .map(|d| d.season)
        .unwrap_or(league.season);
    let player = &mut league.players[player_index];
    player.status = PlayerStatus::Active;
    player.team_id = team_id.to_string();
    player.contract = Some(contract);
    if kind == DraftKind::Rookie {
        player.draft_year = Some(season);
    }
    let name = player.name.clone();
    league.teams[team_index].roster.push(player_id.to_string());
    let pick = DraftPick {
        number,
        team_id: team_id.to_string(),
        player_id: player_id.to_string(),
    };
    if let Some(draft) = league.draft.as_mut() {
        draft.picks.push(pick.clone());
    }
    let label = team_label(league, team_id);
    league.transactions.push(format!(
        "{label} drafted {name} (#{number}, {}, {}y)",
        money(contract.salary),
        contract.years_left
    ));
    finalize_if_done(league);
    Ok(pick)
}

fn finalize_if_done(league: &mut League) {
    let done = league.draft.as_ref().is_some_and(|d| d.complete());
    if done && league.phase == Phase::FantasyDraft {
        league.phase = Phase::RegularSeason;
    }
}

/// Cheap deterministic noise in -1..1 from a seed (splitmix64).
fn unit_noise(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// Per-team facts the AI needs, computed once per pick.
struct TeamContext {
    payroll: u32,
    roster_len: usize,
    position_counts: [usize; 5],
    sigma: f64,
    salt: u64,
}

fn position_slot(position: Position) -> usize {
    match position {
        Position::PG => 0,
        Position::SG => 1,
        Position::SF => 2,
        Position::PF => 3,
        Position::C => 4,
    }
}

fn team_context(league: &League, team_id: &str) -> Option<TeamContext> {
    let team = league.teams.iter().find(|team| team.id == team_id)?;
    let mut position_counts = [0usize; 5];
    for id in &team.roster {
        if let Some(player) = league.players.iter().find(|p| &p.id == id) {
            position_counts[position_slot(player.position)] += 1;
        }
    }
    // GM quality: most front offices are decent, a few are hopeless.
    let gm = (stable_hash(team_id) % 1000) as f64 / 1000.0;
    Some(TeamContext {
        payroll: payroll(league, team_id),
        roster_len: team.roster.len(),
        position_counts,
        sigma: 1.5 + 16.0 * gm * gm,
        salt: league.seed ^ stable_hash(team_id),
    })
}

/// How the AI values a prospect/free agent for a team; `None` = cannot pick.
fn ai_score(kind: DraftKind, ctx: &TeamContext, player: &Player, pick_number: u16) -> Option<f64> {
    // Taste noise: front offices disagree about players.
    let noise = unit_noise(ctx.salt ^ stable_hash(&player.id) ^ pick_number as u64) * ctx.sigma;
    match kind {
        DraftKind::Rookie => Some(board_value(DraftKind::Rookie, player) + noise),
        DraftKind::Fantasy => {
            let salary = market_salary(player);
            let still_needed = FANTASY_ROUNDS.saturating_sub(ctx.roster_len + 1) as u32;
            if ctx.payroll + salary + still_needed * MIN_SALARY > SALARY_CAP {
                return None;
            }
            let overall = player_overall(player) as f64;
            let upside = if player.age <= 23 {
                (player.potential as f64 - overall).max(0.0) * 0.12
            } else {
                0.0
            };
            let counts = &ctx.position_counts;
            let same = counts[position_slot(player.position)] as f64;
            let mut need = 0.0;
            if ctx.roster_len >= 4 {
                let guards = counts[0] + counts[1];
                let bigs = counts[3] + counts[4];
                if guards == 0 && matches!(player.position, Position::PG | Position::SG) {
                    need += 6.0;
                }
                if bigs == 0 && matches!(player.position, Position::PF | Position::C) {
                    need += 6.0;
                }
            }
            let crowding = (same - 3.0).max(0.0) * 7.0;
            Some(overall + upside - 0.25 * salary as f64 / 1000.0 - crowding + need + noise)
        }
    }
}

pub fn ai_choice(league: &League, team_id: &str) -> Option<String> {
    let draft = league.draft.as_ref()?;
    let number = draft.picks.len() as u16 + 1;
    let ctx = team_context(league, team_id)?;
    league
        .players
        .iter()
        .filter(|player| is_available(draft, player))
        .filter_map(|player| {
            ai_score(draft.kind, &ctx, player, number).map(|score| (score, &player.id))
        })
        .max_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.cmp(a.1))
        })
        .map(|(_, id)| id.clone())
}

/// Auto-pick for whoever is on the clock (used for AI teams and "auto").
pub fn auto_pick(league: &mut League) -> Result<DraftPick, DraftError> {
    let team_id = league
        .draft
        .as_ref()
        .ok_or(DraftError::NoDraft)?
        .on_the_clock()
        .cloned()
        .ok_or(DraftError::Complete)?;
    let mut choice = ai_choice(league, &team_id);
    if choice.is_none() {
        // Cap-strapped fantasy team: take the cheapest available body.
        choice = available_players(league)
            .into_iter()
            .min_by_key(|player| (market_salary(player), player.id.clone()))
            .map(|player| player.id.clone());
    }
    let player_id = choice.ok_or(DraftError::NotAvailable)?;
    make_pick(league, &team_id, &player_id)
}

/// Run AI picks until a human-owned team is on the clock or the draft ends.
/// Returns how many picks were made.
pub fn run_ai_until_human(league: &mut League) -> usize {
    let mut made = 0;
    loop {
        let Some(team_id) = league
            .draft
            .as_ref()
            .and_then(|draft| draft.on_the_clock().cloned())
        else {
            return made;
        };
        let human = league
            .teams
            .iter()
            .any(|team| team.id == team_id && team.owner_user_id.is_some());
        if human || auto_pick(league).is_err() {
            return made;
        }
        made += 1;
    }
}

/// Auto-draft every remaining pick, humans included.
pub fn run_all(league: &mut League) -> usize {
    let mut made = 0;
    while auto_pick(league).is_ok() {
        made += 1;
    }
    made
}
