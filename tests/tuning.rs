//! Diagnostic: league-wide box-score averages. Run with
//! `cargo test --test tuning -- --ignored --nocapture`.

use basketballman::generator::generate_league;
use basketballman::sim::{SimConfig, simulate_game};

#[test]
#[ignore]
fn print_league_box_score_averages() {
    let mut league = generate_league(7);
    let ids: Vec<String> = league
        .schedule
        .iter()
        .take(400)
        .map(|g| g.id.clone())
        .collect();
    let (mut pts, mut fga, mut fgm, mut tpa, mut tpm, mut fta, mut ftm) = (0u32, 0, 0, 0, 0, 0, 0);
    let (mut reb, mut ast, mut stl, mut blk, mut tov, mut pf, mut teams) = (0u32, 0, 0, 0, 0, 0, 0);
    let mut max_min = 0;
    let mut fouled_out = 0;
    let mut minutes_by_rank: Vec<(u32, u32)> = vec![(0, 0); 12];
    for id in &ids {
        let r = simulate_game(&mut league, id, SimConfig::default()).unwrap();
        for team in [
            &league
                .schedule
                .iter()
                .find(|g| &g.id == id)
                .unwrap()
                .home_team_id,
            &league
                .schedule
                .iter()
                .find(|g| &g.id == id)
                .unwrap()
                .away_team_id,
        ] {
            teams += 1;
            let mut lines: Vec<_> = r
                .player_stats
                .as_ref()
                .unwrap()
                .iter()
                .filter(|l| &l.team_id == team)
                .collect();
            for l in &lines {
                pts += l.points as u32;
                fga += l.field_goals_attempted as u32;
                fgm += l.field_goals_made as u32;
                tpa += l.three_pointers_attempted as u32;
                tpm += l.three_pointers_made as u32;
                fta += l.free_throws_attempted as u32;
                ftm += l.free_throws_made as u32;
                reb += l.rebounds as u32;
                ast += l.assists as u32;
                stl += l.steals as u32;
                blk += l.blocks as u32;
                tov += l.turnovers as u32;
                pf += l.fouls as u32;
                max_min = max_min.max(l.minutes);
                if l.fouls >= 6 {
                    fouled_out += 1;
                }
            }
            lines.sort_by_key(|l| std::cmp::Reverse(l.minutes));
            for (i, l) in lines.iter().enumerate() {
                minutes_by_rank[i].0 += l.minutes as u32;
                minutes_by_rank[i].1 += 1;
            }
        }
    }
    let t = teams as f64;
    println!("games={} team-games={teams}", ids.len());
    println!(
        "PTS {:.1} FG {:.1}-{:.1} ({:.3}) 3P {:.1}-{:.1} ({:.3}) FT {:.1}-{:.1}",
        pts as f64 / t,
        fgm as f64 / t,
        fga as f64 / t,
        fgm as f64 / fga as f64,
        tpm as f64 / t,
        tpa as f64 / t,
        tpm as f64 / tpa as f64,
        ftm as f64 / t,
        fta as f64 / t
    );
    println!(
        "REB {:.1} AST {:.1} STL {:.1} BLK {:.1} TOV {:.1} PF {:.1}",
        reb as f64 / t,
        ast as f64 / t,
        stl as f64 / t,
        blk as f64 / t,
        tov as f64 / t,
        pf as f64 / t
    );
    println!(
        "max minutes {max_min}, fouled out {fouled_out} ({:.2}/team-game)",
        fouled_out as f64 / t
    );
    let line: Vec<String> = minutes_by_rank
        .iter()
        .map(|(m, c)| format!("{:.1}", *m as f64 / (*c).max(1) as f64))
        .collect();
    println!("minutes by rank: {}", line.join(" "));
}

#[test]
#[ignore]
fn print_minutes_mode_rotation() {
    use basketballman::models::LineupMode;
    use basketballman::sim::player_overall;
    let mut league = generate_league(7);
    let game = league.schedule[0].clone();
    let idx = league
        .teams
        .iter()
        .position(|t| t.id == game.home_team_id)
        .unwrap();
    let mut roster: Vec<_> = league.teams[idx]
        .roster
        .iter()
        .filter_map(|id| league.players.iter().find(|p| &p.id == id))
        .cloned()
        .collect();
    roster.sort_by_key(|p| (player_overall(p), p.id.clone()));
    let starters: Vec<String> = roster.iter().take(5).map(|p| p.id.clone()).collect();
    let team = &mut league.teams[idx];
    team.starters = starters.clone();
    team.lineup_mode = Some(LineupMode::Minutes);
    for s in &starters {
        team.minute_targets.insert(s.clone(), 40);
    }
    team.minute_targets
        .insert(roster.last().unwrap().id.clone(), 0);
    let r = simulate_game(&mut league, &game.id, SimConfig::default()).unwrap();
    for l in r
        .player_stats
        .unwrap()
        .iter()
        .filter(|l| l.team_id == game.home_team_id)
    {
        let p = roster.iter().find(|p| p.id == l.player_id).unwrap();
        println!(
            "{} ovr {} start {} min {} pf {}",
            p.name,
            player_overall(p),
            starters.contains(&p.id),
            l.minutes,
            l.fouls
        );
    }
}

#[test]
#[ignore]
fn print_auto_rotation_game0() {
    use basketballman::sim::player_overall;
    let mut league = generate_league(7);
    let game = league.schedule[0].clone();
    let r = simulate_game(&mut league, &game.id, SimConfig::default()).unwrap();
    for l in r.player_stats.as_ref().unwrap() {
        let p = league.players.iter().find(|p| p.id == l.player_id).unwrap();
        println!(
            "{} {} {:?} ovr {} end {} min {} pf {} pm {}",
            l.team_id,
            p.name,
            p.position,
            player_overall(p),
            p.ratings.endurance,
            l.minutes,
            l.fouls,
            l.plus_minus
        );
    }
    for e in r
        .play_by_play
        .unwrap()
        .iter()
        .filter(|e| e.description.contains("Owen Pierce"))
        .take(40)
    {
        println!("Q{} {} {}", e.quarter, e.clock, e.description);
    }
}

#[test]
#[ignore]
fn print_ironman_quarters() {
    use basketballman::models::{CHART_BLOCKS, LineupMode};
    use basketballman::sim::{PossessionEngine, player_overall, simulation_input};
    let mut league = generate_league(7);
    let game = league.schedule[0].clone();
    let home = game.home_team_id.clone();
    let mut roster: Vec<_> = league
        .teams
        .iter()
        .find(|t| t.id == home)
        .unwrap()
        .roster
        .iter()
        .map(|id| league.players.iter().find(|p| &p.id == id).unwrap().clone())
        .collect();
    roster.sort_by_key(|p| std::cmp::Reverse(player_overall(p)));
    let five: Vec<String> = roster.iter().take(5).map(|p| p.id.clone()).collect();
    let team = league.teams.iter_mut().find(|t| t.id == home).unwrap();
    team.lineup_mode = Some(LineupMode::Chart);
    team.chart = vec![five.clone(); CHART_BLOCKS];
    team.coach.blowout_bench = false;
    team.coach.closers = false;
    let mut made = [0u32; 4];
    let mut att = [0u32; 4];
    for seed in 0..50 {
        let mut input = simulation_input(&league, &game, SimConfig::default()).unwrap();
        input.seed = seed;
        let r = PossessionEngine.simulate(&input);
        for l in r
            .player_stats
            .as_ref()
            .unwrap()
            .iter()
            .filter(|l| five.contains(&l.player_id))
        {
            if seed == 0 {
                println!(
                    "{} min {} fga {} fgm {}",
                    l.player_id, l.minutes, l.field_goals_attempted, l.field_goals_made
                );
            }
        }
        for e in r.play_by_play.as_ref().unwrap() {
            if e.team_id != home {
                continue;
            }
            let q = e.quarter as usize - 1;
            if e.description.contains("makes two") || e.description.contains("makes three") {
                made[q] += 1;
                att[q] += 1;
            } else if e.description.contains("misses") || e.description.contains("blocks") {
                att[q] += 1;
            }
        }
    }
    println!("made {:?} att {:?}", made, att);
}

#[test]
#[ignore]
fn print_foul_outs() {
    let mut league = generate_league(7);
    let ids: Vec<String> = league
        .schedule
        .iter()
        .take(60)
        .map(|g| g.id.clone())
        .collect();
    let mut shown = 0;
    for id in ids {
        let r = simulate_game(&mut league, &id, SimConfig::default()).unwrap();
        for l in r
            .player_stats
            .as_ref()
            .unwrap()
            .iter()
            .filter(|l| l.fouls >= 6)
        {
            let p = league.players.iter().find(|p| p.id == l.player_id).unwrap();
            if shown < 3 {
                shown += 1;
                println!("--- {} min {} pf {}", p.name, l.minutes, l.fouls);
                for e in r.play_by_play.as_ref().unwrap() {
                    if e.description.contains(&p.name)
                        && (e.description.contains("foul")
                            || e.description.contains("Substitution"))
                    {
                        println!(
                            "Q{} {} {} [{}-{}]",
                            e.quarter, e.clock, e.description, e.away_score, e.home_score
                        );
                    }
                }
            }
        }
    }
}
