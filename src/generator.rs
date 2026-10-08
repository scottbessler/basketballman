use crate::config::{DEFAULT_SEASON, FANTASY_ROUNDS, PROSPECT_CLASS_SIZE, ROSTER_MAX, TEAM_SEEDS};
use crate::draft::{run_all, start_fantasy_draft};
use crate::models::{League, Phase, Team};
use crate::pool::{generate_pool, generate_prospects};
use crate::schedule::generate_schedule;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::{BTreeMap, BTreeSet};

/// How a new league's rosters come about.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StartMode {
    /// The AI runs a fantasy draft instantly.
    Quick,
    /// Owners (and the AI for unclaimed teams) draft the initial rosters.
    Fantasy,
}

/// A ready-to-play league: NBA-shaped players drafted onto 32 teams.
pub fn generate_league(seed: u64) -> League {
    generate_league_with(seed, StartMode::Quick)
}

pub fn generate_league_with(seed: u64, mode: StartMode) -> League {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let teams: Vec<Team> = TEAM_SEEDS
        .iter()
        .enumerate()
        .map(|(index, team_seed)| Team {
            id: format!("t{:02}", index + 1),
            city: team_seed.city.to_string(),
            name: team_seed.name.to_string(),
            conference: team_seed.conference,
            division: team_seed.division,
            roster: Vec::new(),
            owner_user_id: None,
            starters: Vec::new(),
            minute_targets: BTreeMap::new(),
            lineup_mode: None,
            bench_order: Vec::new(),
            chart: Vec::new(),
            strategy: Default::default(),
            coach: Default::default(),
        })
        .collect();

    let mut used_names = BTreeSet::new();
    let pool_size = TEAM_SEEDS.len() * FANTASY_ROUNDS + 60;
    let mut players = generate_pool(&mut rng, pool_size, 1, &mut used_names);
    let prospects = generate_prospects(
        &mut rng,
        PROSPECT_CLASS_SIZE,
        players.len() + 1,
        DEFAULT_SEASON,
        &mut used_names,
    );
    players.extend(prospects);

    let schedule = generate_schedule(DEFAULT_SEASON, &teams);
    let mut league = League {
        id: format!("league-{seed}"),
        name: "Basketballman Association".to_string(),
        seed,
        season: DEFAULT_SEASON,
        teams,
        players,
        schedule,
        results: BTreeMap::new(),
        trades: Vec::new(),
        playoffs: None,
        phase: Phase::RegularSeason,
        draft: None,
        fa_day: 0,
        history: Vec::new(),
        transactions: Vec::new(),
    };
    start_fantasy_draft(&mut league);
    if mode == StartMode::Quick {
        run_all(&mut league);
        league.draft = None;
        league.transactions.clear();
    }
    debug_assert!(league.teams.iter().all(|t| t.roster.len() <= ROSTER_MAX));
    league
}
