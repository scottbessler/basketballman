use crate::models::{GameStatus, League, PlayerGameStats, PlayerId, PlayerSeasonStats, TeamId};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRecord {
    pub team_id: TeamId,
    pub wins: u16,
    pub losses: u16,
    pub points_for: u16,
    pub points_against: u16,
}

impl TeamRecord {
    pub fn pct(&self) -> String {
        let games = self.wins + self.losses;
        if games == 0 {
            ".000".to_string()
        } else {
            format!("{:.3}", self.wins as f64 / games as f64)
                .trim_start_matches('0')
                .to_string()
        }
    }

    pub fn differential(&self) -> i16 {
        self.points_for as i16 - self.points_against as i16
    }
}

pub fn standings(league: &League) -> BTreeMap<TeamId, TeamRecord> {
    let mut records: BTreeMap<TeamId, TeamRecord> = league
        .teams
        .iter()
        .map(|team| {
            (
                team.id.clone(),
                TeamRecord {
                    team_id: team.id.clone(),
                    wins: 0,
                    losses: 0,
                    points_for: 0,
                    points_against: 0,
                },
            )
        })
        .collect();

    for result in league.results.values() {
        let Some(game) = league
            .schedule
            .iter()
            .find(|game| game.id == result.game_id)
        else {
            continue;
        };
        add_result(
            records.get_mut(&game.home_team_id).expect("home record"),
            result.home_score,
            result.away_score,
        );
        add_result(
            records.get_mut(&game.away_team_id).expect("away record"),
            result.away_score,
            result.home_score,
        );
    }

    records
}

fn add_result(record: &mut TeamRecord, own_score: u16, other_score: u16) {
    if own_score > other_score {
        record.wins += 1;
    } else {
        record.losses += 1;
    }
    record.points_for += own_score;
    record.points_against += other_score;
}

pub fn player_season_stats(league: &League) -> BTreeMap<PlayerId, PlayerSeasonStats> {
    let mut totals: BTreeMap<PlayerId, PlayerSeasonStats> = league
        .players
        .iter()
        .map(|player| {
            (
                player.id.clone(),
                PlayerSeasonStats {
                    player_id: player.id.clone(),
                    ..PlayerSeasonStats::default()
                },
            )
        })
        .collect();

    for result in league.results.values() {
        let Some(player_stats) = &result.player_stats else {
            continue;
        };
        for line in player_stats {
            add_player_line(totals.get_mut(&line.player_id).expect("player total"), line);
        }
    }

    totals
}

fn add_player_line(total: &mut PlayerSeasonStats, line: &PlayerGameStats) {
    total.games += 1;
    total.minutes += line.minutes;
    total.points += line.points;
    total.rebounds += line.rebounds;
    total.assists += line.assists;
    total.steals += line.steals;
    total.blocks += line.blocks;
    total.turnovers += line.turnovers;
    total.fouls += line.fouls;
    total.field_goals_attempted += line.field_goals_attempted;
    total.field_goals_made += line.field_goals_made;
    total.three_pointers_attempted += line.three_pointers_attempted;
    total.three_pointers_made += line.three_pointers_made;
    total.free_throws_attempted += line.free_throws_attempted;
    total.free_throws_made += line.free_throws_made;
}

pub fn next_unplayed_date_indices(league: &League, limit: usize) -> Vec<u16> {
    let mut dates = Vec::new();
    for game in &league.schedule {
        if game.status != GameStatus::Scheduled || dates.contains(&game.date_index) {
            continue;
        }
        dates.push(game.date_index);
        if dates.len() == limit {
            break;
        }
    }
    dates
}
