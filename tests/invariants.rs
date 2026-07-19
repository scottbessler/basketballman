use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum_extra::extract::cookie::Key;
use basketballman::config::{NBA_NICKNAMES, TEAM_SEEDS};
use basketballman::generator::generate_league;
use basketballman::models::{Conference, GameStatus, League, TradeStatus};
use basketballman::models::{GameResult, PlayerGameStats};
use basketballman::playoffs::{advance_playoff_day, champion, regular_season_complete};
use basketballman::repo::LeagueRepository;
use basketballman::routes::{AppState, app};
use basketballman::sim::{
    PossessionEngine, SimConfig, player_overall, simulate_game, simulation_input, team_rating,
};
use basketballman::stats::{player_season_stats, standings};
use basketballman::trades::{accept_trade, validate_offer};
use basketballman::users::UserStore;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;
use webauthn_rs::WebauthnBuilder;
use webauthn_rs::prelude::Url;

#[test]
fn default_league_shape_and_names_match_spec() {
    let league = generate_league(42);

    assert_eq!(league.teams.len(), 32);
    assert_eq!(
        league
            .teams
            .iter()
            .filter(|team| team.conference == Conference::East)
            .count(),
        16
    );
    assert_eq!(
        league
            .teams
            .iter()
            .filter(|team| team.conference == Conference::West)
            .count(),
        16
    );

    let expected_cities: BTreeSet<_> = TEAM_SEEDS.iter().map(|team| team.city).collect();
    let actual_cities: BTreeSet<_> = league.teams.iter().map(|team| team.city.as_str()).collect();
    assert_eq!(actual_cities, expected_cities);

    for team in &league.teams {
        assert!(!NBA_NICKNAMES.contains(&team.name.as_str()));
        assert!(team.roster.len() >= 12);
    }
}

#[test]
fn generated_players_have_local_names_and_stable_seed() {
    let left = generate_league(99);
    let right = generate_league(99);
    let other = generate_league(100);

    assert_eq!(left, right);
    assert_ne!(left.players[0].name, other.players[0].name);
    for player in &left.players {
        assert!(!player.name.trim().is_empty());
        assert!(player.name.split_whitespace().count() >= 2);
    }
}

#[test]
fn generated_schedule_has_valid_games_and_76_per_team() {
    let league = generate_league(7);
    let team_ids: BTreeSet<_> = league.teams.iter().map(|team| team.id.as_str()).collect();
    let game_ids: BTreeSet<_> = league
        .schedule
        .iter()
        .map(|game| game.id.as_str())
        .collect();
    let mut counts: BTreeMap<&str, usize> = team_ids.iter().map(|id| (*id, 0)).collect();
    let mut same_conference_counts: BTreeMap<&str, usize> =
        team_ids.iter().map(|id| (*id, 0)).collect();
    let mut other_conference_counts: BTreeMap<&str, usize> =
        team_ids.iter().map(|id| (*id, 0)).collect();
    let mut teams_by_date: BTreeMap<u16, BTreeSet<&str>> = BTreeMap::new();

    assert_eq!(league.schedule.len(), 1216);
    assert_eq!(game_ids.len(), league.schedule.len());
    assert!(league.results.is_empty());

    for game in &league.schedule {
        let home = league
            .teams
            .iter()
            .find(|team| team.id == game.home_team_id)
            .unwrap();
        let away = league
            .teams
            .iter()
            .find(|team| team.id == game.away_team_id)
            .unwrap();
        assert_ne!(game.home_team_id, game.away_team_id);
        assert!(team_ids.contains(game.home_team_id.as_str()));
        assert!(team_ids.contains(game.away_team_id.as_str()));
        assert_eq!(game.status, GameStatus::Scheduled);
        let date_teams = teams_by_date.entry(game.date_index).or_default();
        assert!(date_teams.insert(game.home_team_id.as_str()));
        assert!(date_teams.insert(game.away_team_id.as_str()));
        *counts.get_mut(game.home_team_id.as_str()).unwrap() += 1;
        *counts.get_mut(game.away_team_id.as_str()).unwrap() += 1;
        if home.conference == away.conference {
            *same_conference_counts
                .get_mut(game.home_team_id.as_str())
                .unwrap() += 1;
            *same_conference_counts
                .get_mut(game.away_team_id.as_str())
                .unwrap() += 1;
        } else {
            *other_conference_counts
                .get_mut(game.home_team_id.as_str())
                .unwrap() += 1;
            *other_conference_counts
                .get_mut(game.away_team_id.as_str())
                .unwrap() += 1;
        }
    }

    assert!(counts.values().all(|count| *count == 76));
    assert!(same_conference_counts.values().all(|count| *count == 60));
    assert!(other_conference_counts.values().all(|count| *count == 16));
    assert_eq!(teams_by_date.len(), 76);
    assert!(teams_by_date.values().all(|teams| teams.len() == 32));
}

#[test]
fn simulation_persists_one_positive_result_winner_and_player_stats() {
    let mut league = generate_league(7);
    let game_id = league.schedule[0].id.clone();
    let home = league.schedule[0].home_team_id.clone();
    let away = league.schedule[0].away_team_id.clone();

    let first = simulate_game(&mut league, &game_id, SimConfig::default()).expect("result");
    let second = simulate_game(&mut league, &game_id, SimConfig::default()).expect("same result");

    assert_eq!(first, second);
    assert_eq!(league.results.len(), 1);
    assert!(first.home_score > 0);
    assert!(first.away_score > 0);
    assert!(first.winner_team_id == home || first.winner_team_id == away);
    assert_eq!(league.schedule[0].status, GameStatus::Played);

    let lines = first.player_stats.as_ref().expect("player stats");
    assert_eq!(lines.len(), 24);
    assert!(lines.iter().all(|line| line.minutes > 0));

    let home_points: u16 = lines
        .iter()
        .filter(|line| line.team_id == home)
        .map(|line| line.points)
        .sum();
    let away_points: u16 = lines
        .iter()
        .filter(|line| line.team_id == away)
        .map(|line| line.points)
        .sum();
    assert_eq!(home_points, first.home_score);
    assert_eq!(away_points, first.away_score);

    let totals = player_season_stats(&league);
    assert!(
        totals
            .values()
            .any(|stats| stats.games == 1 && stats.points > 0)
    );
    let records = standings(&league);
    assert_eq!(records.values().map(|record| record.wins).sum::<u16>(), 1);
    assert_eq!(records.values().map(|record| record.losses).sum::<u16>(), 1);
}

#[test]
fn possession_minutes_follow_on_floor_rotation() {
    let mut league = generate_league(7);
    let game_id = league.schedule[0].id.clone();
    let game = league.schedule[0].clone();
    let result = simulate_game(&mut league, &game_id, SimConfig::default()).expect("result");
    let lines = result.player_stats.as_ref().expect("player stats");

    for team_id in [&game.home_team_id, &game.away_team_id] {
        let team = league
            .teams
            .iter()
            .find(|team| &team.id == team_id)
            .expect("team");
        let team_lines: Vec<_> = lines
            .iter()
            .filter(|line| &line.team_id == team_id)
            .collect();
        let minutes: u16 = team_lines.iter().map(|line| line.minutes).sum();
        assert!(
            (239..=241).contains(&minutes),
            "{team_id} has {minutes} minutes"
        );
        assert!(team_lines.iter().all(|line| line.minutes <= 48));

        let mut players: Vec<_> = team
            .roster
            .iter()
            .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
            .collect();
        players.sort_by_key(|player| {
            (
                std::cmp::Reverse(player_overall(player)),
                player.id.as_str(),
            )
        });
        for player in players.into_iter().take(5) {
            let minutes = team_lines
                .iter()
                .find(|line| line.player_id == player.id)
                .expect("top-five line")
                .minutes;
            assert!(
                (28..=40).contains(&minutes),
                "{} has {minutes} minutes",
                player.name
            );
        }
    }
}

#[test]
fn possession_engine_is_pure_over_scheduled_game_input() {
    let league = generate_league(7);
    let game = league.schedule[0].clone();
    let input = simulation_input(&league, &game, SimConfig::default()).expect("input");
    let possession = PossessionEngine;

    let possession_first = possession.simulate(&input);
    let possession_second = possession.simulate(&input);
    let possession_result = possession.simulate(&input);

    assert_eq!(possession_first, possession_second);
    assert!(league.results.is_empty());
    assert_eq!(league.schedule[0].status, GameStatus::Scheduled);
    assert_valid_engine_result(&game.home_team_id, &game.away_team_id, &possession_result);
    assert!(possession_result.team_stats.unwrap().possessions > 0);
}

#[test]
fn possession_engine_conserves_plus_minus_by_team() {
    let league = generate_league(7);
    let game = league.schedule[0].clone();
    let input = simulation_input(&league, &game, SimConfig::default()).expect("input");

    let result = PossessionEngine.simulate(&input);
    assert_plus_minus_invariant(&game.home_team_id, &game.away_team_id, &result);
}

#[test]
fn player_game_stats_backcompat_defaults_plus_minus() {
    let value = serde_json::json!({
        "player_id": "p001",
        "team_id": "t01",
        "minutes": 24,
        "points": 12,
        "rebounds": 4,
        "assists": 3,
        "steals": 1,
        "blocks": 0,
        "turnovers": 2,
        "fouls": 1,
        "field_goals_attempted": 10,
        "field_goals_made": 5,
        "three_pointers_attempted": 3,
        "three_pointers_made": 1,
        "free_throws_attempted": 2,
        "free_throws_made": 1
    });

    let stats: PlayerGameStats = serde_json::from_value(value).expect("legacy stats");
    assert_eq!(stats.plus_minus, 0);
}

#[test]
fn repository_roundtrip_preserves_ids_and_results() {
    let path = temp_path("roundtrip.json");
    let repo = LeagueRepository::new(&path);
    let mut league = repo.load_or_generate(123).expect("generate");
    let first_team_id = league.teams[0].id.clone();
    let game_id = league.schedule[0].id.clone();
    simulate_game(&mut league, &game_id, SimConfig::default()).expect("simulate");
    repo.save(&league).expect("save");

    let loaded = repo.load().expect("load");
    assert_eq!(loaded.teams[0].id, first_team_id);
    assert!(loaded.results.contains_key(&game_id));

    let _ = std::fs::remove_file(path);
}

#[test]
fn repository_reset_preserves_ids_and_clears_games() {
    let path = temp_path("reset.json");
    let repo = LeagueRepository::new(&path);
    let mut league = repo.load_or_generate(123).expect("generate");
    let first_team_id = league.teams[0].id.clone();
    let first_player_id = league.players[0].id.clone();
    let first_game_id = league.schedule[0].id.clone();
    simulate_game(&mut league, &first_game_id, SimConfig::default()).expect("simulate");

    repo.reset(&mut league).expect("reset");

    assert_eq!(league.teams[0].id, first_team_id);
    assert_eq!(league.players[0].id, first_player_id);
    assert_eq!(league.schedule[0].id, first_game_id);
    assert!(league.results.is_empty());
    assert!(
        league
            .schedule
            .iter()
            .all(|game| game.status == GameStatus::Scheduled)
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn repository_regen_creates_fresh_empty_league() {
    let path = temp_path("regen.json");
    let repo = LeagueRepository::new(&path);
    let league = repo.load_or_generate(123).expect("generate");
    let old_seed = league.seed;
    let old_first_player = league.players[0].name.clone();

    let next = repo.regenerate(old_seed).expect("regen");

    assert_ne!(next.seed, old_seed);
    assert_ne!(next.players[0].name, old_first_player);
    assert_eq!(next.teams.len(), 32);
    assert_eq!(next.schedule.len(), 1216);
    assert!(next.results.is_empty());

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn ssr_routes_work_without_javascript_and_sim_ranges_persist() {
    let path = temp_path("web.json");
    let repo = LeagueRepository::new(&path);
    let league = repo.load_or_generate(7).expect("generate");
    let game_id = league.schedule[0].id.clone();
    let app = app(test_state(repo.clone(), league));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/standings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("<table"));
    assert!(html.contains("Sim Day"));
    assert!(html.contains("/static/islands.js"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/games/{game_id}/simulate"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let loaded = repo.load().expect("load");
    assert!(loaded.results.contains_key(&game_id));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/games/{game_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Box Score"));
    assert!(html.contains("PTS"));

    let player_id = loaded.players[0].id.clone();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/players/{player_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sim/week")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let loaded = repo.load().expect("load");
    assert!(loaded.results.len() > 1);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/league/reset")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let loaded = repo.load().expect("load");
    assert!(loaded.results.is_empty());

    let old_seed = loaded.seed;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/league/regen")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let loaded = repo.load().expect("load");
    assert_ne!(loaded.seed, old_seed);
    assert!(loaded.results.is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn team_overall_ratings_form_wide_bell_curve() {
    for seed in [7u64, 42, 99] {
        let league = generate_league(seed);
        let ratings: Vec<i16> = league
            .teams
            .iter()
            .map(|team| team_rating(&league, team))
            .collect();
        let min = *ratings.iter().min().unwrap();
        let max = *ratings.iter().max().unwrap();
        let mean: f64 = ratings.iter().map(|r| f64::from(*r)).sum::<f64>() / 32.0;
        assert!(
            (58..=68).contains(&min),
            "seed {seed}: min rating {min} out of range"
        );
        assert!(
            (76..=86).contains(&max),
            "seed {seed}: max rating {max} out of range"
        );
        assert!(
            max - min >= 12,
            "seed {seed}: spread {} too small",
            max - min
        );
        assert!(
            (69.0..=75.0).contains(&mean),
            "seed {seed}: mean rating {mean} out of range"
        );
    }
}

fn play_regular_season(league: &mut League) {
    let game_ids: Vec<String> = league.schedule.iter().map(|game| game.id.clone()).collect();
    for game_id in game_ids {
        simulate_game(league, &game_id, SimConfig::default());
    }
}

#[test]
fn playoffs_run_best_of_seven_to_a_champion() {
    let mut league = generate_league(7);
    assert!(!regular_season_complete(&league));
    assert!(!advance_playoff_day(&mut league, SimConfig::default()));

    play_regular_season(&mut league);
    assert!(regular_season_complete(&league));

    let mut days = 0;
    while advance_playoff_day(&mut league, SimConfig::default()) {
        days += 1;
        assert!(days < 200, "playoffs should terminate");
    }

    let playoffs = league.playoffs.as_ref().expect("playoffs");
    assert_eq!(playoffs.rounds.len(), 4);
    assert_eq!(playoffs.rounds[0].series.len(), 8);
    assert_eq!(playoffs.rounds[1].series.len(), 4);
    assert_eq!(playoffs.rounds[2].series.len(), 2);
    assert_eq!(playoffs.rounds[3].series.len(), 1);

    for round in &playoffs.rounds {
        for series in &round.series {
            let winner_wins = series.high_wins.max(series.low_wins);
            let loser_wins = series.high_wins.min(series.low_wins);
            assert_eq!(winner_wins, 4, "series should end at 4 wins");
            assert!(loser_wins < 4);
            assert_eq!(
                series.game_ids.len(),
                (winner_wins + loser_wins) as usize,
                "one game per series win"
            );
            assert!(series.winner_team_id.is_some());
            for game_id in &series.game_ids {
                assert!(league.results.contains_key(game_id));
            }
        }
    }

    let first_round_teams: BTreeSet<&str> = playoffs.rounds[0]
        .series
        .iter()
        .flat_map(|series| [series.high_team_id.as_str(), series.low_team_id.as_str()])
        .collect();
    assert_eq!(first_round_teams.len(), 16);
    for conference in [Conference::East, Conference::West] {
        let count = playoffs.rounds[0]
            .series
            .iter()
            .filter(|series| series.conference == Some(conference))
            .count();
        assert_eq!(count, 4, "{conference:?} should have 4 first-round series");
    }

    let champ = champion(&league).expect("champion");
    assert!(league.teams.iter().any(|team| team.id == champ));
}

#[test]
fn accepted_trade_swaps_rosters_and_updates_players() {
    let mut league = generate_league(7);
    let user_a = uuid::Uuid::new_v4();
    let user_b = uuid::Uuid::new_v4();
    league.teams[0].owner_user_id = Some(user_a);
    league.teams[1].owner_user_id = Some(user_b);
    let offered = league.teams[0].roster[0].clone();
    let requested = league.teams[1].roster[0].clone();
    league.teams[0].starters = league.teams[0].roster[..5].to_vec();
    league.teams[0].minute_targets.insert(offered.clone(), 36);

    let from = league.teams[0].id.clone();
    let to = league.teams[1].id.clone();
    validate_offer(
        &league,
        &from,
        &to,
        std::slice::from_ref(&offered),
        std::slice::from_ref(&requested),
    )
    .expect("valid offer");
    // Offering a player the team does not have is rejected.
    assert!(
        validate_offer(
            &league,
            &from,
            &to,
            std::slice::from_ref(&requested),
            std::slice::from_ref(&offered),
        )
        .is_err()
    );

    league.trades.push(basketballman::models::TradeOffer {
        id: "tr0001".to_string(),
        from_team_id: from.clone(),
        to_team_id: to.clone(),
        offered_player_ids: vec![offered.clone()],
        requested_player_ids: vec![requested.clone()],
        note: Some("deal?".to_string()),
        status: TradeStatus::Pending,
        response_note: None,
        counter_of: None,
    });
    accept_trade(&mut league, "tr0001").expect("accept");

    assert_eq!(league.trades[0].status, TradeStatus::Accepted);
    assert!(!league.teams[0].roster.contains(&offered));
    assert!(league.teams[0].roster.contains(&requested));
    assert!(league.teams[1].roster.contains(&offered));
    assert!(!league.teams[1].roster.contains(&requested));
    assert!(!league.teams[0].starters.contains(&offered));
    assert!(!league.teams[0].minute_targets.contains_key(&offered));
    let moved = league
        .players
        .iter()
        .find(|player| player.id == offered)
        .unwrap();
    assert_eq!(moved.team_id, to);

    // A second acceptance of the same trade is rejected.
    assert!(accept_trade(&mut league, "tr0001").is_err());
}

#[test]
fn custom_starters_and_minute_targets_shape_rotation() {
    let mut league = generate_league(7);
    let game = league.schedule[0].clone();
    let home_id = game.home_team_id.clone();
    let team_index = league
        .teams
        .iter()
        .position(|team| team.id == home_id)
        .unwrap();

    let mut roster: Vec<_> = league.teams[team_index]
        .roster
        .iter()
        .filter_map(|player_id| league.players.iter().find(|player| &player.id == player_id))
        .collect();
    roster.sort_by_key(|player| (player_overall(player), player.id.clone()));
    // Start the five worst players and give the single worst a heavy load.
    let starters: Vec<String> = roster
        .iter()
        .take(5)
        .map(|player| player.id.clone())
        .collect();
    let workhorse = starters[0].clone();
    let benched = roster.last().unwrap().id.clone();

    let team = &mut league.teams[team_index];
    team.starters = starters.clone();
    for starter in &starters {
        team.minute_targets.insert(starter.clone(), 40);
    }
    team.minute_targets.insert(benched.clone(), 0);
    let _ = workhorse;

    let result = simulate_game(&mut league, &game.id, SimConfig::default()).expect("result");
    let lines = result.player_stats.as_ref().expect("player stats");
    let team_lines: Vec<_> = lines
        .iter()
        .filter(|line| line.team_id == home_id)
        .collect();

    let starter_minutes: u16 = team_lines
        .iter()
        .filter(|line| starters.contains(&line.player_id))
        .map(|line| line.minutes)
        .sum();
    let benched_minutes: u16 = team_lines
        .iter()
        .filter(|line| line.player_id == benched)
        .map(|line| line.minutes)
        .sum();
    assert!(
        starter_minutes > 140,
        "chosen starters should dominate minutes, got {starter_minutes}"
    );
    assert!(
        benched_minutes < 12,
        "benched player should barely play, got {benched_minutes}"
    );
    let total: u16 = team_lines.iter().map(|line| line.minutes).sum();
    assert!((239..=241).contains(&total), "total minutes {total}");
}

fn test_state(repo: LeagueRepository, league: League) -> AppState {
    let users_dir = temp_path("users");
    std::fs::create_dir_all(&users_dir).unwrap();
    let rp_origin = Url::parse("http://localhost:3000").unwrap();
    let webauthn = WebauthnBuilder::new("localhost", &rp_origin)
        .unwrap()
        .build()
        .unwrap();
    AppState {
        repo,
        league: Arc::new(Mutex::new(league)),
        users: Arc::new(UserStore::load(&users_dir).unwrap()),
        webauthn: Arc::new(webauthn),
        key: Key::generate(),
        passkey_disabled: true,
    }
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("basketballman-{nanos}-{name}"))
}

fn assert_valid_engine_result(home: &str, away: &str, result: &GameResult) {
    assert!(result.home_score > 0);
    assert!(result.away_score > 0);
    assert!(result.winner_team_id == home || result.winner_team_id == away);
    let lines = result.player_stats.as_ref().expect("player stats");
    assert_eq!(lines.len(), 24);
    let home_points: u16 = lines
        .iter()
        .filter(|line| line.team_id == home)
        .map(|line| line.points)
        .sum();
    let away_points: u16 = lines
        .iter()
        .filter(|line| line.team_id == away)
        .map(|line| line.points)
        .sum();
    assert_eq!(home_points, result.home_score);
    assert_eq!(away_points, result.away_score);
}

fn assert_plus_minus_invariant(home: &str, away: &str, result: &GameResult) {
    let lines = result.player_stats.as_ref().expect("player stats");
    let home_plus_minus: i16 = lines
        .iter()
        .filter(|line| line.team_id == home)
        .map(|line| line.plus_minus)
        .sum();
    let away_plus_minus: i16 = lines
        .iter()
        .filter(|line| line.team_id == away)
        .map(|line| line.plus_minus)
        .sum();
    let margin = result.home_score as i16 - result.away_score as i16;
    assert_eq!(home_plus_minus, 5 * margin);
    assert_eq!(away_plus_minus, -5 * margin);
    assert_eq!(home_plus_minus + away_plus_minus, 0);
}
