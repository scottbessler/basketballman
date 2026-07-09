use crate::config::TEAM_SEEDS;
use crate::generator::generate_league;
use crate::models::{GameStatus, League};
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
        Ok(serde_json::from_str(&body)?)
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
        && league.schedule.len() == 1216
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
