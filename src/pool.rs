//! NBA-shaped player generation: a few superstars, a deep middle of
//! starters and role players, realistic ages and positions, archetypes that
//! tilt ratings (sharpshooter, rim protector, floor general, ...), and a
//! development curve so young players carry potential.

use crate::config::{FIRST_NAMES, LAST_NAMES};
use crate::models::{Player, PlayerStatus, Position, Ratings};
use crate::sim::player_overall;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeSet;

/// Rating slots: 0 2pt%, 1 3pt%, 2 ft%, 3 inside, 4 three tendency,
/// 5 passing, 6 handling, 7 perimeter D, 8 interior D, 9 steal, 10 block,
/// 11 off reb, 12 def reb, 13 endurance.
type Vector = [f64; 14];

/// Typical profile by position at an average overall.
fn position_flavor(position: Position) -> Vector {
    let base: [f64; 14] = match position {
        Position::C => [
            55., 30., 66., 82., 22., 42., 38., 34., 88., 30., 92., 88., 92., 54.,
        ],
        Position::PF => [
            54., 33., 70., 70., 34., 52., 48., 48., 72., 42., 72., 78., 82., 58.,
        ],
        Position::SF => [
            53., 35., 76., 58., 50., 62., 60., 62., 58., 56., 48., 58., 64., 62.,
        ],
        Position::SG => [
            52., 38., 82., 44., 68., 52., 70., 68., 38., 68., 28., 38., 46., 64.,
        ],
        Position::PG => [
            50., 37., 84., 38., 74., 84., 86., 78., 28., 78., 22., 25., 35., 66.,
        ],
    };
    base
}

struct Archetype {
    name: &'static str,
    tilt: &'static [(usize, f64)],
}

fn archetypes(position: Position) -> &'static [Archetype] {
    match position {
        Position::PG => &[
            Archetype {
                name: "Floor General",
                tilt: &[(5, 14.0), (6, 8.0), (3, -6.0)],
            },
            Archetype {
                name: "Scoring Guard",
                tilt: &[(1, 8.0), (4, 8.0), (3, 4.0), (5, -8.0)],
            },
            Archetype {
                name: "Defensive Guard",
                tilt: &[(9, 14.0), (7, 12.0), (1, -4.0)],
            },
        ],
        Position::SG => &[
            Archetype {
                name: "Sharpshooter",
                tilt: &[(1, 10.0), (4, 10.0), (2, 6.0), (3, -10.0)],
            },
            Archetype {
                name: "Slasher",
                tilt: &[(3, 12.0), (6, 6.0), (4, -10.0)],
            },
            Archetype {
                name: "3-and-D",
                tilt: &[(7, 12.0), (9, 8.0), (1, 4.0), (5, -6.0)],
            },
        ],
        Position::SF => &[
            Archetype {
                name: "Scoring Wing",
                tilt: &[(3, 6.0), (1, 4.0), (0, 2.0)],
            },
            Archetype {
                name: "3-and-D Wing",
                tilt: &[(7, 12.0), (1, 6.0), (3, -8.0)],
            },
            Archetype {
                name: "Point Forward",
                tilt: &[(5, 14.0), (6, 8.0), (11, -6.0)],
            },
        ],
        Position::PF => &[
            Archetype {
                name: "Stretch Four",
                tilt: &[(1, 14.0), (4, 14.0), (10, -10.0), (11, -6.0)],
            },
            Archetype {
                name: "Bruiser",
                tilt: &[(3, 8.0), (11, 10.0), (12, 6.0), (4, -10.0)],
            },
            Archetype {
                name: "Two-Way Forward",
                tilt: &[(7, 8.0), (8, 6.0), (9, 6.0)],
            },
        ],
        Position::C => &[
            Archetype {
                name: "Rim Protector",
                tilt: &[(10, 14.0), (8, 10.0), (4, -6.0)],
            },
            Archetype {
                name: "Stretch Five",
                tilt: &[(1, 16.0), (4, 16.0), (10, -10.0), (8, -6.0)],
            },
            Archetype {
                name: "Post Scorer",
                tilt: &[(3, 10.0), (0, 4.0), (2, 4.0)],
            },
            Archetype {
                name: "Glass Cleaner",
                tilt: &[(11, 10.0), (12, 10.0)],
            },
        ],
    }
}

/// How far below peak a player of this age is, before potential scaling.
/// Mirrors real aging curves: fast growth through 22-23, peak ~25-28, decline
/// from ~29 that steepens after 33.
pub fn development_gap(age: u8) -> f64 {
    match age {
        0..=19 => 15.0,
        20 => 12.0,
        21 => 9.0,
        22 => 7.0,
        23 => 5.0,
        24 => 3.0,
        25 => 1.5,
        26 => 0.5,
        27 | 28 => 0.0,
        29 => 0.8,
        30 => 1.8,
        31 => 3.0,
        32 => 4.5,
        33 => 6.0,
        34 => 8.0,
        35 => 10.0,
        36 => 12.0,
        _ => 14.0,
    }
}

/// Young high-potential players start further below their peak.
pub fn gap_for(age: u8, peak: f64) -> f64 {
    let base = development_gap(age);
    if age >= 27 {
        base
    } else {
        base * (1.0 + (peak - 60.0).max(0.0) / 50.0)
    }
}

/// NBA-ish age mix for rostered players.
const AGE_WEIGHTS: &[(u8, f64)] = &[
    (19, 2.0),
    (20, 4.0),
    (21, 6.0),
    (22, 8.0),
    (23, 9.0),
    (24, 9.0),
    (25, 9.0),
    (26, 8.0),
    (27, 8.0),
    (28, 7.0),
    (29, 6.0),
    (30, 5.0),
    (31, 4.0),
    (32, 3.0),
    (33, 2.0),
    (34, 1.5),
    (35, 1.0),
    (36, 0.5),
];

const POSITIONS: [Position; 5] = [
    Position::PG,
    Position::SG,
    Position::SF,
    Position::PF,
    Position::C,
];

fn weighted_age(rng: &mut ChaCha8Rng) -> u8 {
    let total: f64 = AGE_WEIGHTS.iter().map(|(_, w)| w).sum();
    let mut ticket = rng.gen_range(0.0..total);
    for (age, weight) in AGE_WEIGHTS {
        if ticket < *weight {
            return *age;
        }
        ticket -= weight;
    }
    25
}

fn unique_name(rng: &mut ChaCha8Rng, used: &mut BTreeSet<String>) -> String {
    for _ in 0..40 {
        let name = format!(
            "{} {}",
            FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())],
            LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())]
        );
        if used.insert(name.clone()) {
            return name;
        }
    }
    let name = format!(
        "{} {}",
        FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())],
        LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())]
    );
    used.insert(name.clone());
    name
}

fn to_ratings(v: &Vector) -> Ratings {
    let skill = |x: f64| x.round().clamp(0.0, 99.0) as u8;
    Ratings {
        two_point_pct: v[0].round().clamp(38.0, 68.0) as u8,
        three_point_pct: v[1].round().clamp(24.0, 48.0) as u8,
        ft_pct: v[2].round().clamp(45.0, 97.0) as u8,
        inside_scoring: skill(v[3]),
        three_tendency: skill(v[4]),
        passing: skill(v[5]),
        ball_handling: skill(v[6]),
        perimeter_defense: skill(v[7]),
        interior_defense: skill(v[8]),
        steal: skill(v[9]),
        block: skill(v[10]),
        offensive_rebounding: skill(v[11]),
        defensive_rebounding: skill(v[12]),
        endurance: v[13].round().clamp(25.0, 95.0) as u8,
    }
}

/// Add `delta` to a rating vector: skills fully, percentages at a third
/// (they have narrower ranges), endurance untouched.
pub fn shift(v: &mut Vector, delta: f64) {
    for (i, value) in v.iter_mut().enumerate() {
        match i {
            0..=2 => *value += delta / 3.5,
            13 => {}
            _ => *value += delta,
        }
    }
}

pub fn vector_of(ratings: &Ratings) -> Vector {
    [
        ratings.two_point_pct as f64,
        ratings.three_point_pct as f64,
        ratings.ft_pct as f64,
        ratings.inside_scoring as f64,
        ratings.three_tendency as f64,
        ratings.passing as f64,
        ratings.ball_handling as f64,
        ratings.perimeter_defense as f64,
        ratings.interior_defense as f64,
        ratings.steal as f64,
        ratings.block as f64,
        ratings.offensive_rebounding as f64,
        ratings.defensive_rebounding as f64,
        ratings.endurance as f64,
    ]
}

pub fn ratings_of(v: &Vector) -> Ratings {
    to_ratings(v)
}

/// Build a player of `position` whose current overall lands on `target`.
fn build_ratings(position: Position, target: f64, rng: &mut ChaCha8Rng) -> (Ratings, &'static str) {
    let mut v = position_flavor(position);
    let options = archetypes(position);
    let archetype = &options[rng.gen_range(0..options.len())];
    for (slot, tilt) in archetype.tilt {
        v[*slot] += tilt;
    }
    // Individual quirks.
    for (i, value) in v.iter_mut().enumerate() {
        let spread = if i <= 2 { 2.5 } else { 7.0 };
        *value += rng.gen_range(-spread..=spread);
    }
    v[13] = (v[13] + rng.gen_range(-12.0..=12.0)).clamp(30.0, 92.0);

    let probe = |v: &Vector| -> f64 {
        let player = Player {
            id: String::new(),
            name: String::new(),
            age: 25,
            position,
            ratings: to_ratings(v),
            team_id: String::new(),
            status: PlayerStatus::FreeAgent,
            potential: 0,
            contract: None,
            scouting_fudge: 0,
            draft_year: None,
            history: Vec::new(),
        };
        player_overall(&player) as f64
    };
    for _ in 0..6 {
        let delta = target - probe(&v);
        if delta.abs() < 0.5 {
            break;
        }
        shift(&mut v, delta);
    }
    (to_ratings(&v), archetype.name)
}

pub struct Spec {
    pub id: String,
    pub age: u8,
    pub peak: f64,
    pub position: Position,
    pub status: PlayerStatus,
}

pub fn make_player(spec: Spec, rng: &mut ChaCha8Rng, used: &mut BTreeSet<String>) -> Player {
    let gap = gap_for(spec.age, spec.peak) + rng.gen_range(-1.5..=1.5);
    let current = (spec.peak - gap).clamp(38.0, 99.0);
    let (ratings, _) = build_ratings(spec.position, current, rng);
    let player = Player {
        id: spec.id,
        name: unique_name(rng, used),
        age: spec.age,
        position: spec.position,
        ratings,
        team_id: String::new(),
        status: spec.status,
        potential: 0,
        contract: None,
        scouting_fudge: 0,
        draft_year: None,
        history: Vec::new(),
    };
    let overall = player_overall(&player) as f64;
    let potential = if spec.age >= 27 {
        overall
    } else {
        (spec.peak + rng.gen_range(-2.0..=2.0)).max(overall)
    };
    Player {
        potential: potential.round().clamp(0.0, 99.0) as u8,
        ..player
    }
}

/// Peak-overall quantile curve for veterans: a long tail of stars.
fn veteran_peak(rng: &mut ChaCha8Rng, age: u8) -> f64 {
    let mut q: f64 = rng.gen_range(0.0..1.0);
    // Survivorship: old players still in the league are mostly good ones.
    if age >= 31 {
        q = q.max(rng.gen_range(0.0..1.0));
    }
    if age >= 34 {
        q = q.max(rng.gen_range(0.0..1.0));
    }
    54.0 + 43.0 * q.powf(2.0)
}

/// A league's worth of players (vets and young players) with ids starting
/// at `first_id`.
pub fn generate_pool(
    rng: &mut ChaCha8Rng,
    count: usize,
    first_id: usize,
    used: &mut BTreeSet<String>,
) -> Vec<Player> {
    (0..count)
        .map(|n| {
            let age = weighted_age(rng);
            let position = POSITIONS[rng.gen_range(0..5)];
            let peak = veteran_peak(rng, age);
            make_player(
                Spec {
                    id: format!("p{:03}", first_id + n),
                    age,
                    peak,
                    position,
                    status: PlayerStatus::FreeAgent,
                },
                rng,
                used,
            )
        })
        .collect()
}

/// A draft class: ages 19-22, a thin top end, scouting uncertainty.
pub fn generate_prospects(
    rng: &mut ChaCha8Rng,
    count: usize,
    first_id: usize,
    season: u16,
    used: &mut BTreeSet<String>,
) -> Vec<Player> {
    (0..count)
        .map(|n| {
            let age = match rng.gen_range(0..100) {
                0..=44 => 19,
                45..=69 => 20,
                70..=84 => 21,
                _ => 22,
            };
            let position = POSITIONS[rng.gen_range(0..5)];
            let q: f64 = rng.gen_range(0.0..1.0);
            let peak = 52.0 + 42.0 * q.powf(2.2);
            let mut player = make_player(
                Spec {
                    id: format!("p{:03}", first_id + n),
                    age,
                    peak,
                    position,
                    status: PlayerStatus::Prospect,
                },
                rng,
                used,
            );
            player.draft_year = Some(season);
            player.scouting_fudge = rng.gen_range(-7..=7);
            player
        })
        .collect()
}
