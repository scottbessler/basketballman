//! Lineup editor: form validation and the owner-only HTTP flow.

mod common;

use axum::http::StatusCode;
use basketballman::generator::generate_league;
use basketballman::lineup::{LineupOutcome, apply_lineup_form};
use basketballman::models::{CHART_BLOCKS, LineupMode};
use common::test_app;

fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn form_sets_starters_bench_minutes_and_sliders() {
    let league = generate_league(7);
    let mut team = league.teams[0].clone();
    let roster = team.roster.clone();
    // Reverse the roster to prove order is taken from the form.
    let mut pairs: Vec<(String, String)> = vec![("mode".into(), "minutes".into())];
    for id in roster.iter().rev() {
        pairs.push(("order".into(), id.clone()));
        pairs.push((format!("min_{id}"), "20".into()));
    }
    pairs.push(("pace".into(), "90".into()));
    pairs.push(("foul_caution".into(), "999".into()));
    pairs.push(("depth".into(), "3".into()));
    pairs.push(("coach_form".into(), "1".into()));
    pairs.push(("closers".into(), "1".into()));

    let outcome = apply_lineup_form(&mut team, &roster, &pairs).expect("valid form");
    assert_eq!(outcome, LineupOutcome::Saved);
    assert_eq!(team.mode(), LineupMode::Minutes);
    let expected: Vec<String> = roster.iter().rev().take(5).cloned().collect();
    assert_eq!(team.starters, expected);
    assert_eq!(team.bench_order.len(), roster.len() - 5);
    assert_eq!(team.strategy.pace, 90);
    // Out-of-range sliders clamp instead of erroring.
    assert_eq!(team.coach.foul_caution, 100);
    assert_eq!(team.coach.depth, 7);
    assert!(team.coach.closers);
    assert!(!team.coach.blowout_bench, "unchecked toggles turn off");
    assert_eq!(team.minute_targets.len(), roster.len());
}

#[test]
fn form_ignores_foreign_players_and_dedupes() {
    let league = generate_league(7);
    let mut team = league.teams[0].clone();
    let roster = team.roster.clone();
    let stranger = league.teams[1].roster[0].clone();
    let pairs = fields(&[
        ("mode", "minutes"),
        ("order", &roster[3]),
        ("order", &roster[3]),
        ("order", &stranger),
    ]);
    apply_lineup_form(&mut team, &roster, &pairs).unwrap();
    assert_eq!(team.starters[0], roster[3]);
    assert!(!team.starters.contains(&stranger));
    let mut all: Vec<_> = team.starters.iter().chain(&team.bench_order).collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), roster.len(), "every roster player listed once");
}

#[test]
fn chart_mode_requires_five_per_block() {
    let league = generate_league(7);
    let mut team = league.teams[0].clone();
    let roster = team.roster.clone();

    let mut bad: Vec<(String, String)> = vec![("mode".into(), "chart".into())];
    for block in 0..CHART_BLOCKS {
        for id in roster.iter().take(if block == 4 { 4 } else { 5 }) {
            bad.push((format!("chart_{block}"), id.clone()));
        }
    }
    let error = apply_lineup_form(&mut team, &roster, &bad).unwrap_err();
    assert!(error.contains("five players"));
    assert!(team.chart.is_empty(), "nothing saved on error");

    let mut good: Vec<(String, String)> = vec![("mode".into(), "chart".into())];
    for block in 0..CHART_BLOCKS {
        for id in roster.iter().take(5) {
            good.push((format!("chart_{block}"), id.clone()));
        }
    }
    apply_lineup_form(&mut team, &roster, &good).unwrap();
    assert_eq!(team.mode(), LineupMode::Chart);
    assert_eq!(team.chart.len(), CHART_BLOCKS);

    // Switching back to auto keeps the chart for later.
    apply_lineup_form(&mut team, &roster, &fields(&[("mode", "auto")])).unwrap();
    assert_eq!(team.mode(), LineupMode::Auto);
    assert_eq!(team.chart.len(), CHART_BLOCKS);
}

#[test]
fn legacy_starter_fields_still_work_and_move_nudges_order() {
    let league = generate_league(7);
    let mut team = league.teams[0].clone();
    let roster = team.roster.clone();
    let pairs: Vec<(String, String)> = roster
        .iter()
        .skip(2)
        .take(5)
        .map(|id| ("starter".to_string(), id.clone()))
        .collect();
    apply_lineup_form(&mut team, &roster, &pairs).unwrap();
    assert_eq!(team.starters, roster[2..7].to_vec());
    assert_eq!(team.mode(), LineupMode::Minutes);

    // Move the 6th man (first bench) up into the starting five.
    let sixth = team.bench_order[0].clone();
    let first = team.starters[0].clone();
    let outcome = apply_lineup_form(
        &mut team,
        &roster,
        &fields(&[("order", &first), ("move", &format!("up:{sixth}"))]),
    )
    .unwrap();
    assert_eq!(outcome, LineupOutcome::Moved);
}

#[test]
fn reset_returns_to_auto_defaults() {
    let league = generate_league(7);
    let mut team = league.teams[0].clone();
    let roster = team.roster.clone();
    team.strategy.pace = 99;
    team.starters = roster[..5].to_vec();
    apply_lineup_form(&mut team, &roster, &fields(&[("reset", "1")])).unwrap();
    assert_eq!(team.mode(), LineupMode::Auto);
    assert_eq!(team.strategy.pace, 50);
    assert!(team.starters.is_empty());
}

#[tokio::test]
async fn editor_is_owner_only_and_saves_each_mode_over_http() {
    let app = test_app(generate_league(7));
    let team_id = "t01";

    // Guests are bounced to the public team page.
    let reply = app.get(&format!("/teams/{team_id}/lineup"), None).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location.as_deref(), Some("/teams/t01"));

    let cookie = app.sign_up("coach").await;
    let reply = app
        .post(&format!("/teams/{team_id}/claim"), Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);

    let page = app
        .get(&format!("/teams/{team_id}/lineup"), Some(&cookie))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    for needle in [
        "data-depth",
        "data-chart",
        "name=\"pace\"",
        "name=\"foul_caution\"",
        "chart_11",
    ] {
        assert!(page.body.contains(needle), "editor missing {needle}");
    }
    // The unsaved chart is the coach's suggestion, labeled as such.
    assert!(page.body.contains("not saved yet"));

    // Another user cannot save onto this team.
    let other = app.sign_up("rival").await;
    let reply = app
        .post_form(
            &format!("/teams/{team_id}/lineup"),
            &[("mode", "auto")],
            Some(&other),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    // Suggest then save as a chart.
    let reply = app
        .post(&format!("/teams/{team_id}/lineup/suggest"), Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let page = app
        .get(&format!("/teams/{team_id}/lineup"), Some(&cookie))
        .await;
    assert!(page.body.contains("Saved chart"));

    let reply = app
        .post_form(
            &format!("/teams/{team_id}/lineup"),
            &[("mode", "chart"), ("pace", "80")],
            Some(&cookie),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    {
        let league = app.state.league.lock().unwrap();
        let team = league.teams.iter().find(|t| t.id == team_id).unwrap();
        assert_eq!(team.mode(), LineupMode::Chart);
        assert_eq!(team.strategy.pace, 80);
    }

    // The saved chart actually drives a simulated game.
    {
        let mut league = app.state.league.lock().unwrap();
        let game = league
            .schedule
            .iter()
            .find(|g| g.home_team_id == team_id || g.away_team_id == team_id)
            .unwrap()
            .id
            .clone();
        let result = basketballman::sim::simulate_game(
            &mut league,
            &game,
            basketballman::sim::SimConfig::default(),
        )
        .unwrap();
        assert!(result.home_score > 0);
    }

    // Invalid chart: bounced back with a message, nothing changed.
    let reply = app
        .post_form(
            &format!("/teams/{team_id}/lineup"),
            &[("mode", "chart"), ("chart_0", "p001")],
            Some(&cookie),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert!(reply.location.unwrap().contains("error="));

    // Team page links to the editor for the owner.
    let team_page = app.get(&format!("/teams/{team_id}"), Some(&cookie)).await;
    assert!(team_page.body.contains("/teams/t01/lineup"));
}
