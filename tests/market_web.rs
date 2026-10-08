//! HTTP flows for free agency, the draft room, the offseason and new leagues.

mod common;

use axum::http::StatusCode;
use basketballman::contracts::{free_agents, payroll};
use basketballman::draft::start_rookie_draft;
use basketballman::generator::generate_league;
use basketballman::models::{Phase, PlayerStatus};
use basketballman::playoffs::{advance_playoff_day, champion, start_playoffs};
use basketballman::sim::{SimConfig, simulate_game};
use common::test_app;

#[tokio::test]
async fn pages_render_for_guests_and_owners() {
    let app = test_app(generate_league(7));
    for path in [
        "/free-agents",
        "/draft",
        "/offseason",
        "/league/new",
        "/teams/t01",
        "/players/p001",
    ] {
        let reply = app.get(path, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
    }
    let draft_page = app.get("/draft", None).await;
    assert!(draft_page.body.contains("Draft Class"));
    assert!(draft_page.body.contains("Scouting grades are estimates"));

    let cookie = app.sign_up("gm").await;
    app.post("/teams/t01/claim", Some(&cookie)).await;
    let team = app.get("/teams/t01", Some(&cookie)).await;
    assert!(team.body.contains("Payroll"));
    assert!(team.body.contains("Waive"));
    let fa = app.get("/free-agents", Some(&cookie)).await;
    assert!(fa.body.contains("Sign"));
}

#[tokio::test]
async fn owner_signs_waives_and_is_stopped_by_the_cap() {
    let app = test_app(generate_league(7));
    let cookie = app.sign_up("gm").await;
    app.post("/teams/t01/claim", Some(&cookie)).await;

    // Anonymous signing bounces to login.
    let anon = app.post("/free-agents/p999/sign", None).await;
    assert_eq!(anon.location.as_deref(), Some("/login"));

    let (target, other) = {
        let league = app.state.league.lock().unwrap();
        let fas = free_agents(&league);
        (fas[0].id.clone(), fas[1].id.clone())
    };
    // Waive someone first so the roster has room under the cap.
    let victim = app.state.league.lock().unwrap().teams[0].roster[0].clone();
    let reply = app
        .post(&format!("/teams/t01/waive/{victim}"), Some(&cookie))
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert!(reply.location.unwrap().contains("message="));

    let reply = app
        .post_form(
            &format!("/free-agents/{target}/sign"),
            &[("years", "2")],
            Some(&cookie),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    {
        let league = app.state.league.lock().unwrap();
        let player = league.players.iter().find(|p| p.id == target).unwrap();
        assert_eq!(player.status, PlayerStatus::Active);
        assert_eq!(player.team_id, "t01");
        assert!(payroll(&league, "t01") <= basketballman::config::SALARY_CAP);
    }

    // Someone else's team cannot waive.
    let other_cookie = app.sign_up("rival").await;
    let forbidden = app
        .post(&format!("/teams/t01/waive/{target}"), Some(&other_cookie))
        .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);

    // Sign with no cap room: refused with an explanation, nothing changes.
    {
        let mut league = app.state.league.lock().unwrap();
        let ids = league.teams[0].roster.clone();
        for id in ids {
            let p = league.players.iter_mut().find(|p| p.id == id).unwrap();
            p.contract.as_mut().unwrap().salary = 12_000;
        }
        // Make the free agent expensive by promoting him to a star.
        let star = league
            .players
            .iter()
            .filter(|p| p.status == PlayerStatus::Active)
            .max_by_key(|p| basketballman::sim::player_overall(p))
            .unwrap()
            .clone();
        let fa = league.players.iter_mut().find(|p| p.id == other).unwrap();
        fa.ratings = star.ratings;
        fa.position = star.position;
    }
    let reply = app
        .post_form(
            &format!("/free-agents/{other}/sign"),
            &[("years", "3")],
            Some(&cookie),
        )
        .await;
    let location = reply.location.unwrap();
    assert!(location.contains("error="), "{location}");
    let league = app.state.league.lock().unwrap();
    assert_eq!(
        league
            .players
            .iter()
            .find(|p| p.id == other)
            .unwrap()
            .status,
        PlayerStatus::FreeAgent
    );
}

#[tokio::test]
async fn offseason_is_gated_until_a_champion_exists_and_blocks_sims() {
    let app = test_app(generate_league(7));
    let cookie = app.sign_up("gm").await;
    let early = app.post("/offseason/advance", Some(&cookie)).await;
    assert!(early.location.unwrap().contains("error="));
    let anon = app.post("/offseason/advance", None).await;
    assert_eq!(anon.location.as_deref(), Some("/login"));

    {
        let mut league = app.state.league.lock().unwrap();
        let ids: Vec<String> = league.schedule.iter().map(|g| g.id.clone()).collect();
        for id in ids {
            simulate_game(&mut league, &id, SimConfig::default());
        }
        assert!(start_playoffs(&mut league));
        let mut guard = 0;
        while champion(&league).is_none() && guard < 200 {
            advance_playoff_day(&mut league, SimConfig::default());
            guard += 1;
        }
    }
    let reply = app.post("/offseason/advance", Some(&cookie)).await;
    assert_eq!(reply.location.as_deref(), Some("/draft"));
    assert_eq!(app.state.league.lock().unwrap().phase, Phase::Draft);

    // The sim buttons now bounce to the offseason page.
    let reply = app.post("/sim/day", None).await;
    assert_eq!(reply.location.as_deref(), Some("/offseason"));

    // Draft room as the team on the clock.
    let clock_team = {
        let league = app.state.league.lock().unwrap();
        league
            .draft
            .as_ref()
            .unwrap()
            .on_the_clock()
            .cloned()
            .unwrap()
    };
    app.post(&format!("/teams/{clock_team}/claim"), Some(&cookie))
        .await;
    let page = app.get("/draft", Some(&cookie)).await;
    assert!(page.body.contains("You are on the clock"));
    assert!(page.body.contains("Auto-pick for me"));
    let prospect = {
        let league = app.state.league.lock().unwrap();
        basketballman::draft::available_players(&league)[0]
            .id
            .clone()
    };
    let pick = app
        .post(&format!("/draft/pick/{prospect}"), Some(&cookie))
        .await;
    assert!(pick.location.unwrap().contains("message="));
    assert_eq!(
        app.state
            .league
            .lock()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .picks
            .len(),
        1
    );

    // Sim to the next human pick: stops right away at my next turn (round 2).
    let sim = app.post("/draft/sim", Some(&cookie)).await;
    assert_eq!(sim.status, StatusCode::SEE_OTHER);
    let on_clock = app
        .state
        .league
        .lock()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .on_the_clock()
        .cloned();
    assert_eq!(on_clock.as_deref(), Some(clock_team.as_str()));

    // Walk the rest of the cycle through the UI actions.
    app.post("/draft/sim-all", Some(&cookie)).await;
    for expected in [Phase::Resign, Phase::FreeAgency] {
        app.post("/offseason/advance", Some(&cookie)).await;
        assert_eq!(app.state.league.lock().unwrap().phase, expected);
    }
    let page = app.get("/offseason", Some(&cookie)).await;
    assert!(page.body.contains("Sim 1 FA day"));
    app.post_form("/offseason/fa-days", &[("days", "7")], Some(&cookie))
        .await;
    assert_eq!(app.state.league.lock().unwrap().fa_day, 7);
    app.post("/offseason/advance", Some(&cookie)).await;
    let league = app.state.league.lock().unwrap();
    assert_eq!(league.phase, Phase::RegularSeason);
    assert_eq!(league.season, 2028);
}

#[tokio::test]
async fn new_league_flow_supports_fantasy_draft() {
    let app = test_app(generate_league(7));
    let cookie = app.sign_up("gm").await;
    let denied = app
        .post_form("/league/new", &[("mode", "fantasy")], None)
        .await;
    assert_eq!(denied.location.as_deref(), Some("/login"));

    let bad = app
        .post_form(
            "/league/new",
            &[("mode", "quick"), ("seed", "abc")],
            Some(&cookie),
        )
        .await;
    assert!(bad.location.unwrap().contains("error="));

    let reply = app
        .post_form(
            "/league/new",
            &[("mode", "fantasy"), ("seed", "5")],
            Some(&cookie),
        )
        .await;
    assert_eq!(reply.location.as_deref(), Some("/draft"));
    {
        let league = app.state.league.lock().unwrap();
        assert_eq!(league.seed, 5);
        assert_eq!(league.phase, Phase::FantasyDraft);
        assert!(league.teams.iter().all(|t| t.roster.is_empty()));
    }
    // Claim the team that picks first and draft for it.
    let first = app
        .state
        .league
        .lock()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .on_the_clock()
        .cloned()
        .unwrap();
    app.post(&format!("/teams/{first}/claim"), Some(&cookie))
        .await;
    let page = app.get("/draft", Some(&cookie)).await;
    assert!(page.body.contains("Fantasy Draft"));
    assert!(page.body.contains("Salary ($M)"));
    assert!(page.body.contains("You are on the clock"));
    app.post("/draft/auto", Some(&cookie)).await;
    assert_eq!(
        app.state
            .league
            .lock()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .picks
            .len(),
        1
    );
    app.post("/draft/sim-all", Some(&cookie)).await;
    {
        let league = app.state.league.lock().unwrap();
        assert_eq!(league.phase, Phase::RegularSeason);
        assert!(league.teams.iter().all(|t| t.roster.len() == 13));
    }
    // Quick start path.
    let reply = app
        .post_form("/league/new", &[("mode", "quick")], Some(&cookie))
        .await;
    assert_eq!(reply.location.as_deref(), Some("/standings"));
    assert_eq!(app.state.league.lock().unwrap().phase, Phase::RegularSeason);
}

#[test]
fn start_rookie_draft_is_available_to_scripts() {
    let mut league = generate_league(7);
    // The season is unplayed, so standings are all zero; the draft still orders.
    start_rookie_draft(&mut league);
    assert_eq!(league.draft.as_ref().unwrap().order.len(), 64);
}
