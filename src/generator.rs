use crate::config::{DEFAULT_SEASON, FIRST_NAMES, LAST_NAMES, ROSTER_SIZE, TEAM_SEEDS};
use crate::models::{League, Player, Position, Ratings, Team};
use crate::schedule::generate_schedule;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeMap;

pub fn generate_league(seed: u64) -> League {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut teams = Vec::with_capacity(TEAM_SEEDS.len());
    let mut players = Vec::with_capacity(TEAM_SEEDS.len() * ROSTER_SIZE);

    for (team_index, team_seed) in TEAM_SEEDS.iter().enumerate() {
        let team_id = format!("t{:02}", team_index + 1);
        let mut roster = Vec::with_capacity(ROSTER_SIZE);

        for roster_index in 0..ROSTER_SIZE {
            let player_id = format!("p{:03}", players.len() + 1);
            roster.push(player_id.clone());
            let position = match roster_index % 5 {
                0 => Position::PG,
                1 => Position::SG,
                2 => Position::SF,
                3 => Position::PF,
                _ => Position::C,
            };
            let talent = rng.gen_range(42..=92i16);
            let flavor = position_flavor(position);
            players.push(Player {
                id: player_id,
                name: random_name(&mut rng),
                age: rng.gen_range(19..=35),
                position,
                ratings: Ratings {
                    two_point_pct: percentage(talent, flavor[0], 42, 62, &mut rng),
                    three_point_pct: percentage(talent, flavor[1], 28, 43, &mut rng),
                    ft_pct: percentage(talent, flavor[2], 55, 95, &mut rng),
                    inside_scoring: skill(talent, flavor[3], &mut rng),
                    three_tendency: skill(talent, flavor[4], &mut rng),
                    passing: skill(talent, flavor[5], &mut rng),
                    ball_handling: skill(talent, flavor[6], &mut rng),
                    perimeter_defense: skill(talent, flavor[7], &mut rng),
                    interior_defense: skill(talent, flavor[8], &mut rng),
                    steal: skill(talent, flavor[9], &mut rng),
                    block: skill(talent, flavor[10], &mut rng),
                    offensive_rebounding: skill(talent, flavor[11], &mut rng),
                    defensive_rebounding: skill(talent, flavor[12], &mut rng),
                },
                team_id: team_id.clone(),
            });
        }

        teams.push(Team {
            id: team_id,
            city: team_seed.city.to_string(),
            name: team_seed.name.to_string(),
            conference: team_seed.conference,
            division: team_seed.division,
            roster,
        });
    }

    let schedule = generate_schedule(DEFAULT_SEASON, &teams);

    League {
        id: format!("league-{seed}"),
        name: "Basketballman Association".to_string(),
        seed,
        season: DEFAULT_SEASON,
        teams,
        players,
        schedule,
        results: BTreeMap::new(),
    }
}

fn position_flavor(position: Position) -> [i16; 13] {
    match position {
        Position::C => [55, 30, 66, 82, 22, 42, 38, 34, 88, 30, 92, 88, 92],
        Position::PF => [54, 33, 70, 70, 34, 52, 48, 48, 72, 42, 72, 78, 82],
        Position::SF => [53, 35, 76, 58, 50, 62, 60, 62, 58, 56, 48, 58, 64],
        Position::SG => [52, 38, 82, 44, 68, 52, 70, 68, 38, 68, 28, 38, 46],
        Position::PG => [50, 37, 84, 38, 74, 84, 86, 78, 28, 78, 22, 25, 35],
    }
}

fn skill(talent: i16, positional: i16, rng: &mut ChaCha8Rng) -> u8 {
    (positional + (talent - 50) / 2 + rng.gen_range(-12..=12)).clamp(0, 99) as u8
}

fn percentage(talent: i16, positional: i16, min: i16, max: i16, rng: &mut ChaCha8Rng) -> u8 {
    (positional + (talent - 50) / 3 + rng.gen_range(-3..=3)).clamp(min, max) as u8
}

fn random_name(rng: &mut ChaCha8Rng) -> String {
    let first = FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())];
    let last = LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())];
    format!("{first} {last}")
}
