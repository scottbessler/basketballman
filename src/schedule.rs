use crate::models::{Conference, Game, GameStatus, Team};

pub fn generate_schedule(season: u16, teams: &[Team]) -> Vec<Game> {
    let mut games = Vec::with_capacity(1216);
    let east = conference_teams(teams, Conference::East);
    let west = conference_teams(teams, Conference::West);

    assert_eq!(east.len(), 16);
    assert_eq!(west.len(), 16);

    push_conference_games(season, &mut games, &east);
    push_conference_games(season, &mut games, &west);
    push_cross_conference_games(season, &mut games, &east, &west);

    for (index, game) in games.iter_mut().enumerate() {
        game.id = format!("g{season}-{:04}", index + 1);
        game.date_index = (index / 16 + 1) as u16;
    }

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

fn push_conference_games(season: u16, games: &mut Vec<Game>, teams: &[&Team]) {
    for left in 0..teams.len() {
        for right in (left + 1)..teams.len() {
            push_game(season, games, &teams[left].id, &teams[right].id);
            push_game(season, games, &teams[right].id, &teams[left].id);
            push_game(season, games, &teams[left].id, &teams[right].id);
            push_game(season, games, &teams[right].id, &teams[left].id);
        }
    }
}

fn push_cross_conference_games(season: u16, games: &mut Vec<Game>, east: &[&Team], west: &[&Team]) {
    for (east_index, east_team) in east.iter().enumerate() {
        for (west_index, west_team) in west.iter().enumerate() {
            if (season as usize + east_index + west_index).is_multiple_of(2) {
                push_game(season, games, &east_team.id, &west_team.id);
            } else {
                push_game(season, games, &west_team.id, &east_team.id);
            }
        }
    }
}

fn push_game(season: u16, games: &mut Vec<Game>, home_team_id: &str, away_team_id: &str) {
    games.push(Game {
        id: String::new(),
        season,
        date_index: 0,
        home_team_id: home_team_id.to_string(),
        away_team_id: away_team_id.to_string(),
        status: GameStatus::Scheduled,
    });
}
