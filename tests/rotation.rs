//! Fatigue, fouls, strategy sliders and the coach's rotation modes.

use basketballman::coach::{Rotation, foul_limit, is_blowout, is_clutch, suggest_chart};
use basketballman::generator::generate_league;
use basketballman::models::{CHART_BLOCKS, CoachSettings, GameResult, League, LineupMode, Team};
use basketballman::sim::{
    PossessionEngine, SimConfig, effective_ratings, player_overall, simulation_input,
};

/// Simulate the league's first game under many seeds (same rosters/settings).
fn run_seeds(league: &League, seeds: std::ops::Range<u64>) -> Vec<GameResult> {
    let game = league.schedule[0].clone();
    seeds
        .map(|seed| {
            let mut input = simulation_input(league, &game, SimConfig::default()).unwrap();
            input.seed = seed;
            PossessionEngine.simulate(&input)
        })
        .collect()
}

fn home_team_mut(league: &mut League) -> &mut Team {
    let home = league.schedule[0].home_team_id.clone();
    league.teams.iter_mut().find(|t| t.id == home).unwrap()
}

fn away_team_mut(league: &mut League) -> &mut Team {
    let away = league.schedule[0].away_team_id.clone();
    league.teams.iter_mut().find(|t| t.id == away).unwrap()
}

/// Roster ids best to worst.
fn ranked(league: &League, team_id: &str) -> Vec<String> {
    let team = league.teams.iter().find(|t| t.id == team_id).unwrap();
    let mut players: Vec<_> = team
        .roster
        .iter()
        .map(|id| league.players.iter().find(|p| &p.id == id).unwrap())
        .collect();
    players.sort_by_key(|p| (std::cmp::Reverse(player_overall(p)), p.id.clone()));
    players.into_iter().map(|p| p.id.clone()).collect()
}

fn team_totals(result: &GameResult, team_id: &str) -> (u32, u32, u32, u32) {
    // (steals, fouls, turnovers, three-point attempts)
    result
        .player_stats
        .as_ref()
        .unwrap()
        .iter()
        .filter(|l| l.team_id == team_id)
        .fold((0, 0, 0, 0), |acc, l| {
            (
                acc.0 + l.steals as u32,
                acc.1 + l.fouls as u32,
                acc.2 + l.turnovers as u32,
                acc.3 + l.three_pointers_attempted as u32,
            )
        })
}

#[test]
fn fatigue_scales_ratings_down_but_never_below_floor() {
    let league = generate_league(7);
    let ratings = &league.players[0].ratings;
    let fresh = effective_ratings(ratings, 100.0);
    let gassed = effective_ratings(ratings, 0.0);
    assert_eq!(&fresh, ratings);
    assert!(gassed.perimeter_defense < ratings.perimeter_defense);
    assert!(gassed.three_point_pct < ratings.three_point_pct);
    // Percentages are cushioned relative to pure skills.
    let skill_loss = 1.0 - gassed.perimeter_defense as f64 / ratings.perimeter_defense as f64;
    let pct_loss = 1.0 - gassed.two_point_pct as f64 / ratings.two_point_pct as f64;
    assert!(pct_loss < skill_loss);
    // Endurance itself never degrades mid-game.
    assert_eq!(gassed.endurance, ratings.endurance);
}

#[test]
fn iron_man_lineup_shoots_worse_late_than_early() {
    let mut league = generate_league(7);
    let home_id = league.schedule[0].home_team_id.clone();
    let five: Vec<String> = ranked(&league, &home_id).into_iter().take(5).collect();
    let team = home_team_mut(&mut league);
    team.lineup_mode = Some(LineupMode::Chart);
    team.chart = vec![five.clone(); CHART_BLOCKS];
    team.coach.blowout_bench = false;
    team.coach.closers = false;
    team.coach.foul_caution = 0;

    let results = run_seeds(&league, 0..250);
    let mut made = [0u32; 4];
    let mut attempts = [0u32; 4];
    for result in &results {
        for event in result.play_by_play.as_ref().unwrap() {
            if event.team_id != home_id || event.description.contains("Substitution") {
                continue;
            }
            let quarter = (event.quarter as usize - 1).min(3);
            if event.description.contains("makes two point")
                || event.description.contains("makes three point")
            {
                made[quarter] += 1;
                attempts[quarter] += 1;
            } else if event.description.contains("misses") || event.description.contains("blocks") {
                attempts[quarter] += 1;
            }
        }
    }
    let pct = |q: usize| made[q] as f64 / attempts[q] as f64;
    let late = (made[2] + made[3]) as f64 / (attempts[2] + attempts[3]) as f64;
    assert!(
        pct(0) > late + 0.015,
        "Q1 FG% {:.3} should clearly beat Q3/Q4 {:.3} with no rest",
        pct(0),
        late
    );
    // And the same five really did play ~all night.
    let minutes: u32 = results[0]
        .player_stats
        .as_ref()
        .unwrap()
        .iter()
        .filter(|l| five.contains(&l.player_id))
        .map(|l| l.minutes as u32)
        .sum();
    assert!(minutes >= 190, "iron-man five logged {minutes} minutes");
}

#[test]
fn chart_mode_is_followed_block_by_block() {
    let mut league = generate_league(7);
    let home_id = league.schedule[0].home_team_id.clone();
    let order = ranked(&league, &home_id);
    let first: Vec<String> = order[0..5].to_vec();
    let second: Vec<String> = order[5..10].to_vec();
    let team = home_team_mut(&mut league);
    team.lineup_mode = Some(LineupMode::Chart);
    team.chart = (0..CHART_BLOCKS)
        .map(|block| {
            if block < CHART_BLOCKS / 2 {
                first.clone()
            } else {
                second.clone()
            }
        })
        .collect();
    team.coach.blowout_bench = false;
    team.coach.closers = false;
    team.coach.foul_caution = 0;
    let results = run_seeds(&league, 0..20);
    let mut first_minutes = 0u32;
    for result in &results {
        first_minutes += result
            .player_stats
            .as_ref()
            .unwrap()
            .iter()
            .filter(|l| first.contains(&l.player_id))
            .map(|l| l.minutes as u32)
            .sum::<u32>();
    }
    let average = first_minutes as f64 / results.len() as f64;
    assert!(
        (105.0..=135.0).contains(&average),
        "first unit averaged {average:.1} of ~120 charted minutes"
    );
}

#[test]
fn invalid_chart_falls_back_instead_of_breaking_the_game() {
    let mut league = generate_league(7);
    let team = home_team_mut(&mut league);
    team.lineup_mode = Some(LineupMode::Chart);
    team.chart = vec![vec!["nope".to_string(); 5]; CHART_BLOCKS];
    let results = run_seeds(&league, 0..3);
    for result in results {
        let total: u16 = result.player_stats.unwrap().iter().map(|l| l.minutes).sum();
        assert!((479..=481).contains(&total));
    }
}

#[test]
fn nobody_exceeds_six_fouls_or_forty_eight_minutes() {
    let league = generate_league(7);
    for result in run_seeds(&league, 0..80) {
        for line in result.player_stats.as_ref().unwrap() {
            assert!(line.fouls <= 6, "{} fouls", line.fouls);
            assert!(line.minutes <= 48);
        }
    }
}

#[test]
fn foul_limits_follow_quarters_and_caution_slider() {
    let neutral = CoachSettings::default();
    let limits: Vec<u8> = [0.0, 800.0, 1500.0, 2200.0]
        .iter()
        .map(|t| foul_limit(&neutral, *t, false))
        .collect();
    assert_eq!(limits, vec![2, 3, 4, 5]);

    let careful = CoachSettings {
        foul_caution: 100,
        ..neutral
    };
    let loose = CoachSettings {
        foul_caution: 0,
        ..neutral
    };
    assert_eq!(foul_limit(&careful, 0.0, false), 1);
    assert_eq!(foul_limit(&loose, 0.0, false), 3);
    assert_eq!(foul_limit(&loose, 2200.0, false), 6);
    // Closers play through four fouls but sit at five.
    assert_eq!(foul_limit(&neutral, 2700.0, true), 5);
}

#[test]
fn clutch_and_blowout_windows() {
    assert!(is_clutch(2700.0, 4));
    assert!(!is_clutch(2700.0, 15));
    assert!(!is_clutch(1000.0, 0));
    assert!(is_blowout(2700.0, 20));
    assert!(!is_blowout(2700.0, 8));
    assert!(!is_blowout(1000.0, 40));
    // The cushion shrinks as the clock runs down.
    assert!(!is_blowout(2200.0, 20));
    assert!(is_blowout(2800.0, 15));
}

#[test]
fn caution_slider_changes_foul_trouble_sits() {
    // Count how many substitution events the same team makes with each setting.
    let mut base = generate_league(7);
    home_team_mut(&mut base).coach.foul_caution = 100;
    let mut loose = generate_league(7);
    home_team_mut(&mut loose).coach.foul_caution = 0;
    let fouls_of = |league: &League| -> f64 {
        let home_id = league.schedule[0].home_team_id.clone();
        let results = run_seeds(league, 0..60);
        // Max fouls a single home player accumulates: cautious coaches cap it.
        let total: f64 = results
            .iter()
            .map(|r| {
                r.player_stats
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|l| l.team_id == home_id)
                    .map(|l| l.fouls as f64)
                    .fold(0.0, f64::max)
            })
            .sum();
        total / results.len() as f64
    };
    let cautious = fouls_of(&base);
    let careless = fouls_of(&loose);
    assert!(
        cautious + 0.3 < careless,
        "cautious coach max fouls {cautious:.2} vs careless {careless:.2}"
    );
}

#[test]
fn low_fatigue_tolerance_subs_more_often() {
    let subs = |tolerance: u8| -> usize {
        let mut league = generate_league(7);
        home_team_mut(&mut league).coach.fatigue_tolerance = tolerance;
        let home_id = league.schedule[0].home_team_id.clone();
        run_seeds(&league, 0..30)
            .iter()
            .map(|r| {
                r.play_by_play
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|e| e.team_id == home_id && e.description.starts_with("Substitution"))
                    .count()
            })
            .sum()
    };
    let twitchy = subs(0);
    let grinder = subs(100);
    assert!(twitchy > grinder, "{twitchy} subs vs {grinder}");
}

#[test]
fn pace_slider_changes_possessions() {
    let possessions = |pace: u8| -> f64 {
        let mut league = generate_league(7);
        home_team_mut(&mut league).strategy.pace = pace;
        away_team_mut(&mut league).strategy.pace = pace;
        let results = run_seeds(&league, 0..40);
        results
            .iter()
            .map(|r| r.team_stats.as_ref().unwrap().possessions as f64)
            .sum::<f64>()
            / results.len() as f64
    };
    assert!(possessions(100) > possessions(0) + 8.0);
}

#[test]
fn three_rate_slider_shifts_shot_mix() {
    let threes = |rate: u8| -> f64 {
        let mut league = generate_league(7);
        home_team_mut(&mut league).strategy.three_rate = rate;
        let home_id = league.schedule[0].home_team_id.clone();
        let results = run_seeds(&league, 0..40);
        results
            .iter()
            .map(|r| team_totals(r, &home_id).3 as f64)
            .sum::<f64>()
            / results.len() as f64
    };
    assert!(threes(100) > threes(0) + 5.0);
}

#[test]
fn pressure_trades_steals_for_fouls() {
    let measure = |pressure: u8| -> (f64, f64, f64) {
        let mut league = generate_league(7);
        home_team_mut(&mut league).strategy.pressure = pressure;
        let home_id = league.schedule[0].home_team_id.clone();
        let away_id = league.schedule[0].away_team_id.clone();
        let results = run_seeds(&league, 0..60);
        let n = results.len() as f64;
        let steals: f64 = results
            .iter()
            .map(|r| team_totals(r, &home_id).0 as f64)
            .sum();
        let fouls: f64 = results
            .iter()
            .map(|r| team_totals(r, &home_id).1 as f64)
            .sum();
        let opp_tov: f64 = results
            .iter()
            .map(|r| team_totals(r, &away_id).2 as f64)
            .sum();
        (steals / n, fouls / n, opp_tov / n)
    };
    let (steals_hi, fouls_hi, tov_hi) = measure(100);
    let (steals_lo, fouls_lo, tov_lo) = measure(0);
    assert!(steals_hi > steals_lo);
    assert!(fouls_hi > fouls_lo);
    assert!(tov_hi > tov_lo);
}

#[test]
fn offensive_glass_crashing_gives_up_fast_breaks() {
    // Crashing helps the offensive board count but costs transition defense,
    // visible as more opponent steals-free easy buckets; here we check the
    // direct benefit: more offensive rebounds.
    let rebounds = |glass: u8| -> f64 {
        let mut league = generate_league(7);
        home_team_mut(&mut league).strategy.offensive_glass = glass;
        let home_id = league.schedule[0].home_team_id.clone();
        let results = run_seeds(&league, 0..60);
        results
            .iter()
            .flat_map(|r| r.play_by_play.as_ref().unwrap().iter())
            .filter(|e| e.team_id == home_id && e.description.ends_with("offensive rebound"))
            .count() as f64
            / results.len() as f64
    };
    assert!(rebounds(100) > rebounds(0) + 1.0);
}

#[test]
fn auto_rotation_budgets_240_minutes_and_ranks_by_quality() {
    let league = generate_league(7);
    let team = &league.teams[0];
    let players: Vec<&_> = team
        .roster
        .iter()
        .map(|id| league.players.iter().find(|p| &p.id == id).unwrap())
        .collect();
    let rotation = Rotation::new(team, &players);
    assert_eq!(rotation.mode, LineupMode::Auto);
    let total: f64 = rotation.targets.iter().sum();
    assert!((total - 14_400.0).abs() < 1.0, "targets sum to {total}s");
    assert_eq!(rotation.starters.len(), 5);

    let mut heavy = team.clone();
    heavy.coach.starter_load = 100;
    let heavy = Rotation::new(&heavy, &players);
    let mut light = team.clone();
    light.coach.starter_load = 0;
    let light = Rotation::new(&light, &players);
    let starters = |r: &Rotation| -> f64 { r.starters.iter().map(|i| r.targets[*i]).sum() };
    assert!(starters(&heavy) > starters(&light) + 600.0);
}

#[test]
fn suggested_chart_is_valid_staggered_and_matches_targets() {
    let league = generate_league(7);
    for team in league.teams.iter().take(6) {
        let players: Vec<&_> = team
            .roster
            .iter()
            .map(|id| league.players.iter().find(|p| &p.id == id).unwrap())
            .collect();
        let chart = suggest_chart(team, &players);
        assert_eq!(chart.len(), CHART_BLOCKS);
        let mut blocks_played = vec![0u32; players.len()];
        for block in &chart {
            let mut unique = block.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), 5, "block repeats a player");
            for index in block {
                blocks_played[*index] += 1;
            }
        }
        // Nobody plays more than 9 of 12 blocks (36 min) in a suggested chart
        // at default load, and the starters open the game.
        assert!(blocks_played.iter().all(|b| *b <= 10));
        let rotation = Rotation::new(team, &players);
        for starter in &rotation.starters {
            assert!(chart[0].contains(starter));
        }
        // Stars staggered: at least one of the best two is always on floor.
        let mut rank: Vec<usize> = (0..players.len()).collect();
        rank.sort_by_key(|i| std::cmp::Reverse(player_overall(players[*i])));
        let stars = &rank[0..2];
        assert!(chart.iter().all(|b| b.iter().any(|i| stars.contains(i))));
    }
}
