use crate::models::{Conference, Division, Game, GameStatus, Team};
use std::collections::{BTreeMap, BTreeSet};

pub fn generate_schedule(season: u16, teams: &[Team]) -> Vec<Game> {
    let bonus_pairs = same_conference_bonus_pairs(teams);
    let mut games = Vec::with_capacity(1230);

    for away_index in 0..teams.len() {
        for home_index in (away_index + 1)..teams.len() {
            let a = &teams[away_index];
            let b = &teams[home_index];
            let count = if a.conference != b.conference {
                2
            } else if a.division == b.division || bonus_pairs.contains(&pair_key(&a.id, &b.id)) {
                4
            } else {
                3
            };
            push_series(season, &mut games, &a.id, &b.id, count);
        }
    }

    for (index, game) in games.iter_mut().enumerate() {
        game.id = format!("g{season}-{:04}", index + 1);
        game.date_index = (index / 15 + 1) as u16;
    }

    games
}

fn push_series(season: u16, games: &mut Vec<Game>, team_a: &str, team_b: &str, count: usize) {
    for game_index in 0..count {
        let a_home = match count {
            2 | 4 => game_index % 2 == 0,
            3 => game_index != 1,
            _ => game_index % 2 == 0,
        };
        let (home_team_id, away_team_id) = if a_home {
            (team_a.to_string(), team_b.to_string())
        } else {
            (team_b.to_string(), team_a.to_string())
        };
        games.push(Game {
            id: String::new(),
            season,
            date_index: 0,
            home_team_id,
            away_team_id,
            status: GameStatus::Scheduled,
        });
    }
}

fn same_conference_bonus_pairs(teams: &[Team]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for conference in [Conference::East, Conference::West] {
        let divisions = conference_divisions(conference);
        let mut degree: BTreeMap<&str, usize> = teams
            .iter()
            .filter(|team| team.conference == conference)
            .map(|team| (team.id.as_str(), 0))
            .collect();

        for left_division_index in 0..divisions.len() {
            for right_division_index in (left_division_index + 1)..divisions.len() {
                let left = division_teams(teams, conference, divisions[left_division_index]);
                let right = division_teams(teams, conference, divisions[right_division_index]);
                assert_eq!(left.len(), 5);
                assert_eq!(right.len(), 5);

                for left_index in 0..left.len() {
                    for offset in 0..3 {
                        let right_index = (left_index + offset) % right.len();
                        let a = left[left_index].id.as_str();
                        let b = right[right_index].id.as_str();
                        out.insert(pair_key(a, b));
                        *degree.get_mut(a).expect("known team id") += 1;
                        *degree.get_mut(b).expect("known team id") += 1;
                    }
                }
            }
        }

        assert!(degree.values().all(|value| *value == 6));
    }
    out
}

fn conference_divisions(conference: Conference) -> [Division; 3] {
    match conference {
        Conference::East => [Division::Atlantic, Division::Central, Division::Southeast],
        Conference::West => [Division::Northwest, Division::Pacific, Division::Southwest],
    }
}

fn division_teams(teams: &[Team], conference: Conference, division: Division) -> Vec<&Team> {
    let mut out: Vec<&Team> = teams
        .iter()
        .filter(|team| team.conference == conference && team.division == division)
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn pair_key(a: &str, b: &str) -> String {
    if a < b {
        format!("{a}:{b}")
    } else {
        format!("{b}:{a}")
    }
}
