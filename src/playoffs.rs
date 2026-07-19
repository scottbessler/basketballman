//! Postseason bracket: the 8 best teams in each conference, best-of-7 series.
//!
//! Round 1 pairs seeds (1v8, 4v5, 3v6, 2v7) inside each conference, winners
//! advance through the bracket to a cross-conference Finals. Series games are
//! appended to the league schedule one date at a time (continuing past the
//! 76-date regular season) and simulated with the regular game engine.

use crate::models::{
    Conference, Game, GameStatus, League, PlayoffRound, PlayoffSeries, Playoffs, TeamId,
};
use crate::sim::{SimConfig, simulate_game};
use crate::stats::standings;

pub const REGULAR_SEASON_DATES: u16 = 76;
const WINS_NEEDED: u8 = 4;
const TEAMS_PER_CONFERENCE: usize = 8;

pub fn regular_season_complete(league: &League) -> bool {
    league
        .schedule
        .iter()
        .filter(|game| game.date_index <= REGULAR_SEASON_DATES)
        .all(|game| game.status == GameStatus::Played)
}

/// Conference seeds (best first) from the regular-season standings.
pub fn conference_seeds(league: &League, conference: Conference) -> Vec<TeamId> {
    let records = standings(league);
    let mut teams: Vec<&TeamId> = league
        .teams
        .iter()
        .filter(|team| team.conference == conference)
        .map(|team| &team.id)
        .collect();
    teams.sort_by(|a, b| {
        let ra = records.get(*a).expect("record");
        let rb = records.get(*b).expect("record");
        rb.wins
            .cmp(&ra.wins)
            .then_with(|| ra.losses.cmp(&rb.losses))
            .then_with(|| rb.differential().cmp(&ra.differential()))
            .then_with(|| a.cmp(b))
    });
    teams
        .into_iter()
        .take(TEAMS_PER_CONFERENCE)
        .cloned()
        .collect()
}

/// Build the round-1 bracket. Requires a finished regular season.
pub fn start_playoffs(league: &mut League) -> bool {
    if league.playoffs.is_some() || !regular_season_complete(league) {
        return false;
    }
    let mut series = Vec::new();
    for conference in [Conference::East, Conference::West] {
        let seeds = conference_seeds(league, conference);
        for (high, low) in [(1usize, 8usize), (4, 5), (3, 6), (2, 7)] {
            series.push(PlayoffSeries {
                id: format!("s{}-r1-{}{}v{}", league.season, conference, high, low),
                conference: Some(conference),
                high_seed: high as u8,
                low_seed: low as u8,
                high_team_id: seeds[high - 1].clone(),
                low_team_id: seeds[low - 1].clone(),
                game_ids: Vec::new(),
                high_wins: 0,
                low_wins: 0,
                winner_team_id: None,
            });
        }
    }
    league.playoffs = Some(Playoffs {
        season: league.season,
        rounds: vec![PlayoffRound {
            number: 1,
            name: "First Round".to_string(),
            series,
        }],
        next_date_index: REGULAR_SEASON_DATES + 1,
    });
    true
}

/// The league champion, once the Finals are decided.
pub fn champion(league: &League) -> Option<TeamId> {
    let playoffs = league.playoffs.as_ref()?;
    let finals = playoffs.rounds.iter().find(|round| round.number == 4)?;
    finals.series.first()?.winner_team_id.clone()
}

/// Simulate one playoff date: every unfinished series in the current round
/// plays its next game. Starts the bracket (or the next round) as needed.
/// Returns false when there is nothing left to simulate.
pub fn advance_playoff_day(league: &mut League, config: SimConfig) -> bool {
    if league.playoffs.is_none() && !start_playoffs(league) {
        return false;
    }
    if champion(league).is_some() {
        return false;
    }
    maybe_advance_round(league);

    let Some(playoffs) = league.playoffs.as_ref() else {
        return false;
    };
    let date_index = playoffs.next_date_index;
    let round_index = playoffs.rounds.len() - 1;
    let pending: Vec<(usize, String, String, String, usize)> = playoffs
        .rounds
        .last()
        .expect("playoff round")
        .series
        .iter()
        .enumerate()
        .filter(|(_, series)| !series.finished())
        .map(|(index, series)| {
            let game_number = series.game_ids.len() + 1;
            let (home, away) = if high_seed_hosts(game_number) {
                (series.high_team_id.clone(), series.low_team_id.clone())
            } else {
                (series.low_team_id.clone(), series.high_team_id.clone())
            };
            (index, series.id.clone(), home, away, game_number)
        })
        .collect();
    if pending.is_empty() {
        return false;
    }

    for (series_index, series_id, home, away, game_number) in pending {
        let game_id = format!("{series_id}-g{game_number}");
        league.schedule.push(Game {
            id: game_id.clone(),
            season: league.season,
            date_index,
            home_team_id: home,
            away_team_id: away,
            status: GameStatus::Scheduled,
        });
        let result = simulate_game(league, &game_id, config).expect("playoff game result");
        let playoffs = league.playoffs.as_mut().expect("playoffs");
        let series = &mut playoffs.rounds[round_index].series[series_index];
        series.game_ids.push(game_id);
        if result.winner_team_id == series.high_team_id {
            series.high_wins += 1;
        } else {
            series.low_wins += 1;
        }
        if series.high_wins == WINS_NEEDED {
            series.winner_team_id = Some(series.high_team_id.clone());
        } else if series.low_wins == WINS_NEEDED {
            series.winner_team_id = Some(series.low_team_id.clone());
        }
    }

    let playoffs = league.playoffs.as_mut().expect("playoffs");
    playoffs.next_date_index += 1;
    maybe_advance_round(league);
    true
}

/// Home court runs 2-2-1-1-1: games 1, 2, 5 and 7 at the higher seed.
fn high_seed_hosts(game_number: usize) -> bool {
    matches!(game_number, 1 | 2 | 5 | 7)
}

/// When the current round is decided, build the next one from its winners.
fn maybe_advance_round(league: &mut League) {
    let records = standings(league);
    let Some(playoffs) = league.playoffs.as_mut() else {
        return;
    };
    let current = playoffs.rounds.last().expect("playoff round");
    if current.number >= 4 || !current.series.iter().all(PlayoffSeries::finished) {
        return;
    }
    let next_number = current.number + 1;
    let name = match next_number {
        2 => "Conference Semifinals",
        3 => "Conference Finals",
        _ => "Finals",
    };
    let winners: Vec<(Option<Conference>, u8, TeamId)> = current
        .series
        .iter()
        .map(|series| {
            let winner = series.winner_team_id.clone().expect("series winner");
            let seed = if winner == series.high_team_id {
                series.high_seed
            } else {
                series.low_seed
            };
            (series.conference, seed, winner)
        })
        .collect();

    let mut series = Vec::new();
    for pair in winners.chunks(2) {
        let (conf_a, seed_a, team_a) = pair[0].clone();
        let (_, seed_b, team_b) = pair[1].clone();
        let a_is_high = if next_number == 4 {
            // Finals: home court to the better regular-season record.
            let ra = records.get(&team_a).expect("record");
            let rb = records.get(&team_b).expect("record");
            (ra.wins, rb.losses, ra.differential()) >= (rb.wins, ra.losses, rb.differential())
        } else {
            seed_a < seed_b
        };
        let ((high_seed, high_team), (low_seed, low_team)) = if a_is_high {
            ((seed_a, team_a), (seed_b, team_b))
        } else {
            ((seed_b, team_b), (seed_a, team_a))
        };
        let conference = if next_number == 4 { None } else { conf_a };
        let label = conference
            .map(|conf| conf.to_string().to_lowercase())
            .unwrap_or_else(|| "finals".to_string());
        series.push(PlayoffSeries {
            id: format!(
                "s{}-r{next_number}-{label}{high_seed}v{low_seed}",
                playoffs.season
            ),
            conference,
            high_seed,
            low_seed,
            high_team_id: high_team,
            low_team_id: low_team,
            game_ids: Vec::new(),
            high_wins: 0,
            low_wins: 0,
            winner_team_id: None,
        });
    }
    playoffs.rounds.push(PlayoffRound {
        number: next_number,
        name: name.to_string(),
        series,
    });
}
