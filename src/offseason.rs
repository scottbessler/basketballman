//! The yearly cycle after the playoffs: draft → re-signing → free agency →
//! next season. Everything here is a pure league mutation so the web layer
//! only has to persist the result.

use crate::config::{MIN_SALARY, PROSPECT_CLASS_SIZE, ROSTER_MAX, ROSTER_MIN, SALARY_CAP};
use crate::contracts::{
    ask_salary, free_agents, market_salary, payroll, preferred_years, release_to_pool,
    roster_by_value, sign_free_agent, team_label,
};
use crate::draft::{run_all, start_rookie_draft};
use crate::models::{
    Contract, League, Phase, PlayerStatus, SeasonLine, SeasonSummary, TradeStatus,
};
use crate::playoffs::champion;
use crate::pool::generate_prospects;
use crate::progression::{progress_player, retirement_chance};
use crate::schedule::generate_schedule;
use crate::sim::player_overall;
use crate::stats::{player_season_stats, standings};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeSet;

const FA_DAYS_TOTAL: u16 = 30;

fn rng_for(league: &League, salt: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(league.seed ^ (league.season as u64) << 20 ^ salt)
}

/// What the "next step" button will do, for the UI.
pub fn next_step_label(league: &League) -> Option<&'static str> {
    match league.phase {
        Phase::FantasyDraft => None,
        Phase::RegularSeason => champion(league).map(|_| "Start the draft"),
        Phase::Draft => Some("Finish draft & start re-signing"),
        Phase::Resign => Some("Open free agency"),
        Phase::FreeAgency => Some("Finish free agency & start next season"),
    }
}

/// Run the next offseason step. Errors explain why it cannot run yet.
pub fn advance_phase(league: &mut League) -> Result<String, String> {
    match league.phase {
        Phase::FantasyDraft => Err("Finish the fantasy draft first.".to_string()),
        Phase::RegularSeason => {
            if champion(league).is_none() {
                return Err("The season is not over: crown a champion first.".to_string());
            }
            archive_season(league);
            start_rookie_draft(league);
            Ok("The draft is open.".to_string())
        }
        Phase::Draft => {
            run_all(league);
            progress_league(league);
            league.phase = Phase::Resign;
            Ok("Draft complete. Players have aged and developed; expiring contracts can be re-signed.".to_string())
        }
        Phase::Resign => {
            open_free_agency(league);
            Ok("Free agency is open.".to_string())
        }
        Phase::FreeAgency => {
            start_next_season(league);
            Ok(format!("Season {} begins.", league.season))
        }
    }
}

/// Record the finished season: champion and per-player career lines.
pub fn archive_season(league: &mut League) {
    let stats = player_season_stats(league);
    let season = league.season;
    let champion_id = champion(league);
    let records = standings(league);
    let best = records
        .values()
        .max_by_key(|record| (record.wins, record.differential()))
        .map(|record| {
            format!(
                "{} ({}-{})",
                team_label(league, &record.team_id),
                record.wins,
                record.losses
            )
        })
        .unwrap_or_default();
    let leader = league
        .players
        .iter()
        .filter_map(|player| stats.get(&player.id).map(|line| (player, line)))
        .filter(|(_, line)| line.games >= 40)
        .max_by_key(|(_, line)| line.points as u32 * 1000 / line.games as u32)
        .map(|(player, line)| {
            format!(
                "{} ({:.1} PPG)",
                player.name,
                line.points as f64 / line.games as f64
            )
        })
        .unwrap_or_default();

    let team_names: Vec<(String, String)> = league
        .teams
        .iter()
        .map(|team| (team.id.clone(), format!("{} {}", team.city, team.name)))
        .collect();
    for player in &mut league.players {
        let Some(line) = stats.get(&player.id) else {
            continue;
        };
        if line.games == 0 || player.history.iter().any(|h| h.season == season) {
            continue;
        }
        let per_game = |total: u16| (total as u32 * 10 / line.games as u32) as u16;
        let team = team_names
            .iter()
            .find(|(id, _)| id == &player.team_id)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        player.history.push(SeasonLine {
            season,
            age: player.age,
            team,
            overall: player_overall(player),
            games: line.games,
            ppg: per_game(line.points),
            rpg: per_game(line.rebounds),
            apg: per_game(line.assists),
        });
    }
    let champion_name = champion_id
        .as_deref()
        .map(|id| team_label(league, id))
        .unwrap_or_default();
    league.history.push(SeasonSummary {
        season,
        champion_team_id: champion_id,
        champion_name,
        best_record: best,
        scoring_leader: leader,
    });
}

/// Age everyone a year, develop or decline, retire the old, and tick
/// contracts down (0 years left = expiring).
pub fn progress_league(league: &mut League) {
    let mut rng = rng_for(league, 0x9_0A);
    let season_now = league.season;
    let mut retired: Vec<usize> = Vec::new();
    for (index, player) in league.players.iter_mut().enumerate() {
        if player.status == PlayerStatus::Retired {
            continue;
        }
        // Next year's draft class has not played a year yet.
        if player.status == PlayerStatus::Prospect {
            continue;
        }
        progress_player(player, &mut rng);
        // A rookie's contract starts next season: do not burn a year of it.
        let just_drafted = player.draft_year == Some(season_now);
        if !just_drafted && let Some(contract) = player.contract.as_mut() {
            contract.years_left = contract.years_left.saturating_sub(1);
        }
        let chance = retirement_chance(player);
        if chance > 0.0 && rng.gen_bool(chance.min(1.0)) {
            retired.push(index);
        }
    }
    // Undrafted prospects from this class become free agents.
    let season = league.season;
    for player in &mut league.players {
        if player.status == PlayerStatus::Prospect && player.draft_year == Some(season) {
            player.status = PlayerStatus::FreeAgent;
            // They have aged and developed like everyone else.
            progress_player(player, &mut rng);
        }
    }
    for index in retired {
        let id = league.players[index].id.clone();
        if let Some(team_index) = league
            .teams
            .iter()
            .position(|team| team.roster.contains(&id))
        {
            let label = team_label(league, &league.teams[team_index].id.clone());
            let name = league.players[index].name.clone();
            league
                .transactions
                .push(format!("{name} retired ({label})"));
            release_to_pool(league, team_index, index, PlayerStatus::Retired);
        } else {
            league.players[index].status = PlayerStatus::Retired;
            league.players[index].contract = None;
            league.players[index].team_id = String::new();
        }
    }
}

/// AI teams keep the expiring players they value; everyone else hits the market.
pub fn open_free_agency(league: &mut League) {
    league.phase = Phase::FreeAgency;
    league.fa_day = 0;
    let mut rng = rng_for(league, 0xFA);
    let team_ids: Vec<(String, bool)> = league
        .teams
        .iter()
        .map(|team| (team.id.clone(), team.owner_user_id.is_some()))
        .collect();
    for (team_id, human) in team_ids {
        let expiring: Vec<String> = roster_by_value(league, &team_id)
            .into_iter()
            .filter(|player| player.contract.is_some_and(|c| c.years_left == 0))
            .map(|player| player.id.clone())
            .collect();
        for player_id in expiring {
            let Some(player_index) = league.players.iter().position(|p| p.id == player_id) else {
                continue;
            };
            let overall = player_overall(&league.players[player_index]);
            let keep_chance = if human {
                0.0
            } else if overall >= 68 {
                0.85
            } else if overall >= 62 {
                0.5
            } else {
                0.15
            };
            let salary = market_salary(&league.players[player_index]);
            let room = SALARY_CAP.saturating_sub(payroll(league, &team_id));
            if rng.gen_bool(keep_chance) && salary <= room {
                let years = preferred_years(&league.players[player_index]);
                let name = league.players[player_index].name.clone();
                league.players[player_index].contract = Some(Contract {
                    salary,
                    years_left: years,
                });
                league
                    .transactions
                    .push(format!("{} re-signed {name}", team_label(league, &team_id)));
            } else {
                let team_index = league
                    .teams
                    .iter()
                    .position(|t| t.id == team_id)
                    .unwrap_or(0);
                release_to_pool(league, team_index, player_index, PlayerStatus::FreeAgent);
            }
        }
    }
}

/// AI teams shop the free-agent pool for a number of days.
pub fn sim_free_agency_days(league: &mut League, days: u16) {
    let mut rng = rng_for(league, 0xDA7 ^ league.fa_day as u64);
    for _ in 0..days {
        league.fa_day += 1;
        let ai_teams: Vec<String> = league
            .teams
            .iter()
            .filter(|team| team.owner_user_id.is_none())
            .map(|team| team.id.clone())
            .collect();
        for team_id in ai_teams {
            let roster_len = league
                .teams
                .iter()
                .find(|team| team.id == team_id)
                .map(|team| team.roster.len())
                .unwrap_or(0);
            if roster_len >= ROSTER_MAX - 1 {
                continue;
            }
            let sign_chance = if roster_len < ROSTER_MIN {
                0.6
            } else if roster_len < 14 {
                0.12
            } else {
                0.0
            };
            if !rng.gen_bool(sign_chance) {
                continue;
            }
            let room = SALARY_CAP.saturating_sub(payroll(league, &team_id));
            let worst_rotation = roster_by_value(league, &team_id)
                .get(9)
                .map(|player| player_overall(player))
                .unwrap_or(0);
            let pick = free_agents(league)
                .into_iter()
                .filter(|player| {
                    let ask = ask_salary(player, league.fa_day);
                    ask <= room || (ask == MIN_SALARY && roster_len < ROSTER_MIN)
                })
                .filter(|player| roster_len < ROSTER_MIN || player_overall(player) > worst_rotation)
                .map(|player| {
                    let ask = ask_salary(player, league.fa_day) as f64;
                    let value = player_overall(player) as f64
                        + (player.potential as f64 - player_overall(player) as f64).max(0.0) * 0.1
                        - ask / 1000.0 * 0.2;
                    (value, player.id.clone(), preferred_years(player))
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((_, player_id, years)) = pick {
                let _ = sign_free_agent(league, &team_id, &player_id, years);
            }
        }
    }
}

/// Bring every team to 12-15 players and under the hard cap.
pub fn enforce_roster_rules(league: &mut League) {
    let team_ids: Vec<String> = league.teams.iter().map(|team| team.id.clone()).collect();
    for team_id in team_ids {
        let team_index = league
            .teams
            .iter()
            .position(|t| t.id == team_id)
            .unwrap_or(0);

        // Too many players: cut the least valuable (ability + upside).
        while league.teams[team_index].roster.len() > ROSTER_MAX {
            let Some(worst) = worst_value_player(league, &team_id) else {
                break;
            };
            release_to_pool(league, team_index, worst, PlayerStatus::FreeAgent);
        }

        // Over the cap: shed the worst contracts first.
        let mut guard = 0;
        while payroll(league, &team_id) > SALARY_CAP
            && league.teams[team_index].roster.len() > ROSTER_MIN
            && guard < 30
        {
            guard += 1;
            let Some(worst) = worst_contract_player(league, &team_id) else {
                break;
            };
            release_to_pool(league, team_index, worst, PlayerStatus::FreeAgent);
        }

        // Still over (every contract is big): renegotiate proportionally.
        let total = payroll(league, &team_id);
        if total > SALARY_CAP {
            let factor = SALARY_CAP as f64 / total as f64;
            let roster = league.teams[team_index].roster.clone();
            for player in league.players.iter_mut().filter(|p| roster.contains(&p.id)) {
                if let Some(contract) = player.contract.as_mut() {
                    contract.salary = ((contract.salary as f64 * factor) as u32).max(MIN_SALARY);
                }
            }
        }

        // Too few: sign the best free agents at the vet minimum.
        while league.teams[team_index].roster.len() < ROSTER_MIN {
            let Some(best) = free_agents(league).first().map(|player| player.id.clone()) else {
                break;
            };
            let Some(player_index) = league.players.iter().position(|p| p.id == best) else {
                break;
            };
            league.players[player_index].status = PlayerStatus::Active;
            league.players[player_index].team_id = team_id.clone();
            league.players[player_index].contract = Some(Contract {
                salary: MIN_SALARY,
                years_left: 1,
            });
            league.teams[team_index].roster.push(best);
        }
    }
}

fn value_of(player: &crate::models::Player) -> f64 {
    let overall = player_overall(player) as f64;
    overall + (player.potential as f64 - overall).max(0.0) * 0.3
}

fn worst_value_player(league: &League, team_id: &str) -> Option<usize> {
    roster_by_value(league, team_id)
        .into_iter()
        .min_by(|a, b| {
            value_of(a)
                .partial_cmp(&value_of(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .and_then(|player| league.players.iter().position(|p| p.id == player.id))
}

/// Overpaid relative to ability: most salary per point above replacement.
fn worst_contract_player(league: &League, team_id: &str) -> Option<usize> {
    roster_by_value(league, team_id)
        .into_iter()
        .filter(|player| player.contract.is_some_and(|c| c.years_left > 0))
        .max_by(|a, b| {
            let cost = |p: &crate::models::Player| {
                p.contract.map(|c| c.salary).unwrap_or(0) as f64 / (value_of(p) - 50.0).max(1.0)
            };
            cost(a)
                .partial_cmp(&cost(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .and_then(|player| league.players.iter().position(|p| p.id == player.id))
}

/// Finish free agency and roll the league into the next season.
pub fn start_next_season(league: &mut League) {
    let remaining = FA_DAYS_TOTAL.saturating_sub(league.fa_day);
    if remaining > 0 {
        sim_free_agency_days(league, remaining);
    }
    enforce_roster_rules(league);

    league.season += 1;
    league.schedule = generate_schedule(league.season, &league.teams);
    league.results.clear();
    league.playoffs = None;
    league.phase = Phase::RegularSeason;
    league.draft = None;
    league.fa_day = 0;
    for trade in &mut league.trades {
        if trade.status == TradeStatus::Pending {
            trade.status = TradeStatus::Withdrawn;
        }
    }
    // Next summer's draft class starts scouting now.
    let mut rng = rng_for(league, 0xC1A55);
    let mut used: BTreeSet<String> = league.players.iter().map(|p| p.name.clone()).collect();
    let prospects = generate_prospects(
        &mut rng,
        PROSPECT_CLASS_SIZE,
        league.players.len() + 1,
        league.season,
        &mut used,
    );
    league.players.extend(prospects);
    let excess = league.transactions.len().saturating_sub(300);
    league.transactions.drain(..excess);
}
