use crate::models::{Conference, Game, GameStatus, Team};

pub fn generate_schedule(season: u16, teams: &[Team]) -> Vec<Game> {
    let mut games = Vec::with_capacity(1216);
    let east = conference_teams(teams, Conference::East);
    let west = conference_teams(teams, Conference::West);
    let mut date_index = 1;

    assert_eq!(east.len(), 16);
    assert_eq!(west.len(), 16);

    for cycle in 0..4 {
        for round in 0..15 {
            push_conference_round(season, date_index, &mut games, &east, round, cycle);
            push_conference_round(season, date_index, &mut games, &west, round, cycle);
            date_index += 1;
        }
    }

    for round in 0..16 {
        push_cross_conference_round(season, date_index, &mut games, &east, &west, round);
        date_index += 1;
    }

    for (index, game) in games.iter_mut().enumerate() {
        game.id = format!("g{season}-{:04}", index + 1);
    }

    debug_assert_eq!(date_index - 1, 76);
    games
}

fn conference_teams(teams: &[Team], conference: Conference) -> Vec<&Team> {
    let mut out: Vec<&Team> = teams
        .iter()
        .filter(|team| team.conference == conference)
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn push_conference_round(
    season: u16,
    date_index: u16,
    games: &mut Vec<Game>,
    teams: &[&Team],
    round: usize,
    cycle: usize,
) {
    for (a, b) in round_pairs(teams, round) {
        if cycle.is_multiple_of(2) {
            push_game(season, date_index, games, &a.id, &b.id);
        } else {
            push_game(season, date_index, games, &b.id, &a.id);
        }
    }
}

fn push_cross_conference_round(
    season: u16,
    date_index: u16,
    games: &mut Vec<Game>,
    east: &[&Team],
    west: &[&Team],
    round: usize,
) {
    for east_index in 0..east.len() {
        let west_index = (east_index + round) % west.len();
        if (season as usize + east_index + round).is_multiple_of(2) {
            push_game(
                season,
                date_index,
                games,
                &east[east_index].id,
                &west[west_index].id,
            );
        } else {
            push_game(
                season,
                date_index,
                games,
                &west[west_index].id,
                &east[east_index].id,
            );
        }
    }
}

fn round_pairs<'a>(teams: &[&'a Team], round: usize) -> Vec<(&'a Team, &'a Team)> {
    let mut positions: Vec<usize> = (0..teams.len()).collect();
    for _ in 0..round {
        let tail = positions.pop().expect("team positions");
        positions.insert(1, tail);
    }

    (0..teams.len() / 2)
        .map(|index| {
            let left = positions[index];
            let right = positions[teams.len() - 1 - index];
            (teams[left], teams[right])
        })
        .collect()
}

fn push_game(
    season: u16,
    date_index: u16,
    games: &mut Vec<Game>,
    home_team_id: &str,
    away_team_id: &str,
) {
    games.push(Game {
        id: String::new(),
        season,
        date_index,
        home_team_id: home_team_id.to_string(),
        away_team_id: away_team_id.to_string(),
        status: GameStatus::Scheduled,
    });
}
