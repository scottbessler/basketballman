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
            players.push(Player {
                id: player_id,
                name: random_name(&mut rng),
                age: rng.gen_range(19..=35),
                position,
                ratings: Ratings {
                    offense: rng.gen_range(45..=99),
                    defense: rng.gen_range(45..=99),
                    shooting: rng.gen_range(45..=99),
                    playmaking: rng.gen_range(45..=99),
                    rebounding: rng.gen_range(45..=99),
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

fn random_name(rng: &mut ChaCha8Rng) -> String {
    let first = FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())];
    let last = LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())];
    format!("{first} {last}")
}
