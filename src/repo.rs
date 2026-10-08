use crate::config::TEAM_SEEDS;
use crate::generator::generate_league;
use crate::models::{Contract, GameStatus, League, PlayerStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct LeagueRepository {
    path: PathBuf,
}

impl LeagueRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_or_generate(&self, seed: u64) -> Result<League, RepoError> {
        if self.path.exists() {
            let league = self.load()?;
            if league_shape_valid(&league) {
                Ok(league)
            } else {
                let league = generate_league(seed);
                self.save(&league)?;
                Ok(league)
            }
        } else {
            let league = generate_league(seed);
            self.save(&league)?;
            Ok(league)
        }
    }

    pub fn load(&self) -> Result<League, RepoError> {
        let body = fs::read_to_string(&self.path)?;
        let mut league: League = serde_json::from_str(&body)?;
        migrate(&mut league);
        Ok(league)
    }

    pub fn save(&self, league: &League) -> Result<(), RepoError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(league)?;
        fs::write(&self.path, body)?;
        Ok(())
    }

    pub fn reset(&self, league: &mut League) -> Result<(), RepoError> {
        league.results.clear();
        league.playoffs = None;
        league
            .schedule
            .retain(|game| game.date_index <= crate::playoffs::REGULAR_SEASON_DATES);
        for game in &mut league.schedule {
            game.status = GameStatus::Scheduled;
        }
        self.save(league)
    }

    pub fn regenerate(&self, current_seed: u64) -> Result<League, RepoError> {
        let league = generate_league(current_seed.wrapping_add(1));
        self.save(&league)?;
        Ok(league)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn league_shape_valid(league: &League) -> bool {
    league.teams.len() == TEAM_SEEDS.len()
        && league
            .schedule
            .iter()
            .filter(|game| game.date_index <= crate::playoffs::REGULAR_SEASON_DATES)
            .count()
            == 1216
        && league
            .schedule
            .iter()
            .all(|game| game.season == league.season)
        && schedule_dates_have_unique_teams(league)
}

fn schedule_dates_have_unique_teams(league: &League) -> bool {
    let mut teams_by_date: BTreeMap<u16, BTreeSet<&str>> = BTreeMap::new();
    for game in &league.schedule {
        let teams = teams_by_date.entry(game.date_index).or_default();
        if !teams.insert(game.home_team_id.as_str()) || !teams.insert(game.away_team_id.as_str()) {
            return false;
        }
    }
    true
}

/// Bring a league saved before contracts, potential and free agency existed
/// up to date: give everyone a contract (scaled to fit the hard cap), set
/// potential, seed a free-agent pool and this summer's draft class.
pub fn migrate(league: &mut League) {
    use crate::config::{MIN_SALARY, PROSPECT_CLASS_SIZE, SALARY_CAP};
    use crate::contracts::{market_salary, payroll, preferred_years};
    use crate::pool::{generate_pool, generate_prospects};
    use crate::sim::player_overall;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    for player in &mut league.players {
        if player.potential == 0 {
            let overall = player_overall(player) as f64;
            let upside = if player.age < 27 {
                crate::pool::development_gap(player.age) * 0.5
            } else {
                0.0
            };
            player.potential = (overall + upside).round().clamp(0.0, 99.0) as u8;
        }
    }
    let needs_contracts = league
        .players
        .iter()
        .any(|player| player.status == PlayerStatus::Active && player.contract.is_none());
    if needs_contracts {
        for index in 0..league.players.len() {
            if league.players[index].status != PlayerStatus::Active
                || league.players[index].contract.is_some()
            {
                continue;
            }
            let salary = market_salary(&league.players[index]);
            let years = preferred_years(&league.players[index]);
            league.players[index].contract = Some(Contract {
                salary,
                years_left: years,
            });
        }
        // Scale down any payroll that would break the hard cap.
        let team_ids: Vec<String> = league.teams.iter().map(|team| team.id.clone()).collect();
        for team_id in team_ids {
            let total = payroll(league, &team_id);
            if total <= SALARY_CAP {
                continue;
            }
            let factor = SALARY_CAP as f64 / total as f64;
            let roster = league
                .teams
                .iter()
                .find(|team| team.id == team_id)
                .map(|team| team.roster.clone())
                .unwrap_or_default();
            for player in league.players.iter_mut().filter(|p| roster.contains(&p.id)) {
                if let Some(contract) = player.contract.as_mut() {
                    contract.salary = ((contract.salary as f64 * factor) as u32).max(MIN_SALARY);
                }
            }
        }
    }
    let mut rng = ChaCha8Rng::seed_from_u64(league.seed ^ 0xA11);
    let mut used: std::collections::BTreeSet<String> =
        league.players.iter().map(|p| p.name.clone()).collect();
    let has_free_agents = league
        .players
        .iter()
        .any(|player| player.status == PlayerStatus::FreeAgent);
    if !has_free_agents {
        let mut pool = generate_pool(&mut rng, 45, league.players.len() + 1, &mut used);
        for player in &mut pool {
            player.status = PlayerStatus::FreeAgent;
        }
        league.players.extend(pool);
    }
    let has_class = league.players.iter().any(|player| {
        player.status == PlayerStatus::Prospect && player.draft_year == Some(league.season)
    });
    if !has_class {
        let class = generate_prospects(
            &mut rng,
            PROSPECT_CLASS_SIZE,
            league.players.len() + 1,
            league.season,
            &mut used,
        );
        league.players.extend(class);
    }
}
