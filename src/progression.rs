//! Yearly player development modelled on real NBA aging curves: rapid gains
//! through ~22, a plateau at 25-28, decline from ~29 that steepens after 33.
//! Athletic ratings (explosion, defense, rebounding, stamina) age earlier and
//! faster than skill ratings (shooting, passing), and a player's `potential`
//! sets how much of the early-career growth he realises.

use crate::models::{Player, Ratings};
use crate::pool::{gap_for, ratings_of, shift, vector_of};
use crate::sim::player_overall;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Expected change in overall from `age` to `age + 1` for a player whose
/// peak (potential) is `peak`.
pub fn expected_delta(age: u8, peak: f64) -> f64 {
    gap_for(age, peak) - gap_for(age.saturating_add(1), peak)
}

fn noise(rng: &mut ChaCha8Rng, scale: f64) -> f64 {
    // Sum of three uniforms: bell-shaped, bounded.
    (rng.gen_range(-1.0..=1.0) + rng.gen_range(-1.0..=1.0) + rng.gen_range(-1.0..=1.0)) / 3.0
        * 2.2
        * scale
}

/// Age one player a year and develop (or decline). Returns the overall change.
pub fn progress_player(player: &mut Player, rng: &mut ChaCha8Rng) -> i16 {
    let before = player_overall(player) as i16;
    let age = player.age;
    let peak = (player.potential as f64).max(before as f64);
    let expected = expected_delta(age, peak);
    // Young players vary a lot year to year; veterans less.
    let scale = if age < 25 {
        1.8
    } else if age < 31 {
        1.2
    } else {
        1.0
    };
    let delta = expected + noise(rng, scale);

    // Split the change between athletic and skill ratings.
    let (athletic_factor, skill_factor) = if delta >= 0.0 {
        if age <= 23 { (0.9, 1.1) } else { (1.0, 1.0) }
    } else if age >= 33 {
        (1.5, 0.6)
    } else if age >= 27 {
        (1.3, 0.7)
    } else {
        (1.0, 1.0)
    };
    let mut v = vector_of(&player.ratings);
    for (slot, value) in v.iter_mut().enumerate() {
        match slot {
            // shooting percentages: skill, narrow range
            0..=2 => *value += delta * skill_factor / 3.5,
            // three-point tendency, passing: skill
            4 | 5 => *value += delta * skill_factor,
            // stamina: peaks mid-20s, then fades
            13 => {
                *value += if age <= 24 {
                    1.0
                } else if age >= 28 {
                    -(0.6 + (age as f64 - 28.0) * 0.25)
                } else {
                    0.0
                }
            }
            _ => *value += delta * athletic_factor,
        }
    }
    player.ratings = ratings_of(&v);
    player.age = age.saturating_add(1);

    // Correct drift from position weighting so the overall lands near target.
    let target = before as f64 + delta;
    for _ in 0..2 {
        let now = player_overall(player) as f64;
        let gap = target - now;
        if gap.abs() < 0.8 {
            break;
        }
        let mut v = vector_of(&player.ratings);
        shift(&mut v, gap * 0.8);
        player.ratings = ratings_of(&v);
    }

    let after = player_overall(player) as i16;
    // Potential drifts with how development goes; it collapses to current
    // ability once the player is past his early twenties.
    player.potential = if player.age >= 27 {
        after.max(0) as u8
    } else {
        let drift = ((after - before) as f64 - expected) * 0.6;
        ((peak + drift).round() as i16).clamp(after, 99) as u8
    };
    after - before
}

/// Chance a player retires after this season at his current age/ability.
pub fn retirement_chance(player: &Player) -> f64 {
    let overall = player_overall(player) as f64;
    match player.age {
        0..=33 => 0.0,
        34..=37 => {
            (((player.age as f64 - 33.0) * 0.10) + (64.0 - overall).max(0.0) * 0.03).clamp(0.0, 0.9)
        }
        _ => 1.0,
    }
}

pub fn ratings_sum(ratings: &Ratings) -> u32 {
    vector_of(ratings).iter().map(|v| *v as u32).sum()
}
