//! Contracts, free agency, progression, drafts and the offseason cycle.

use basketballman::config::{
    FANTASY_ROUNDS, MAX_SALARY, MIN_SALARY, PROSPECT_CLASS_SIZE, ROSTER_MAX, ROSTER_MIN, SALARY_CAP,
};
use basketballman::contracts::{
    ContractError, ask_salary, free_agents, market_salary, payroll, resign_player, sign_free_agent,
    waive_player,
};
use basketballman::draft::{
    DraftError, auto_pick, available_players, make_pick, rookie_order, run_ai_until_human, run_all,
};
use basketballman::generator::{StartMode, generate_league, generate_league_with};
use basketballman::models::{League, Phase, PlayerStatus, Position};
use basketballman::offseason::{advance_phase, enforce_roster_rules, sim_free_agency_days};
use basketballman::playoffs::{advance_playoff_day, champion, start_playoffs};
use basketballman::pool::{development_gap, generate_pool};
use basketballman::progression::{progress_player, retirement_chance};
use basketballman::sim::{SimConfig, player_overall, simulate_game};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeSet;

fn assert_league_is_legal(league: &League) {
    for team in &league.teams {
        let n = team.roster.len();
        assert!(
            (ROSTER_MIN..=ROSTER_MAX).contains(&n),
            "{} has {n} players",
            team.id
        );
        assert!(
            payroll(league, &team.id) <= SALARY_CAP,
            "{} payroll {} over cap",
            team.id,
            payroll(league, &team.id)
        );
        for id in &team.roster {
            let player = league.players.iter().find(|p| &p.id == id).unwrap();
            assert_eq!(player.status, PlayerStatus::Active, "{id}");
            assert_eq!(player.team_id, team.id);
            let contract = player.contract.expect("rostered player has a contract");
            assert!((MIN_SALARY..=MAX_SALARY).contains(&contract.salary));
        }
    }
    // Every player is on at most one roster.
    let mut seen = BTreeSet::new();
    for team in &league.teams {
        for id in &team.roster {
            assert!(seen.insert(id.clone()), "{id} on two rosters");
        }
    }
}

#[test]
fn new_league_is_nba_shaped_legal_and_deterministic() {
    let league = generate_league(7);
    assert_league_is_legal(&league);
    assert_eq!(league, generate_league(7), "same seed, same league");

    let mut overalls: Vec<u16> = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Active)
        .map(player_overall)
        .collect();
    overalls.sort_unstable_by(|a, b| b.cmp(a));
    // A few superstars, a thin All-Star tier, then a long tail of role players.
    assert!(overalls[0] >= 88, "best player {}", overalls[0]);
    assert!(overalls[9] >= 84 && overalls[9] <= 94);
    assert!(overalls[99] < overalls[9] - 6);
    assert!(overalls[overalls.len() / 2] < overalls[29] - 12);

    let ages: Vec<f64> = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Active)
        .map(|p| p.age as f64)
        .collect();
    let mean_age = ages.iter().sum::<f64>() / ages.len() as f64;
    assert!((24.5..=28.0).contains(&mean_age), "mean age {mean_age}");
    assert!(ages.iter().cloned().fold(0.0, f64::max) >= 33.0);

    for position in [
        Position::PG,
        Position::SG,
        Position::SF,
        Position::PF,
        Position::C,
    ] {
        let count = league
            .players
            .iter()
            .filter(|p| p.position == position && p.status == PlayerStatus::Active)
            .count();
        assert!(count >= 50, "{position}: {count}");
    }

    // Stars are paid like stars and the top contract is the max or close.
    let top_salary = league
        .players
        .iter()
        .filter_map(|p| p.contract)
        .map(|c| c.salary)
        .max()
        .unwrap();
    assert!(top_salary >= 38_000, "top salary {top_salary}");

    let names: BTreeSet<&String> = league.players.iter().map(|p| &p.name).collect();
    assert_eq!(names.len(), league.players.len(), "player names are unique");
}

#[test]
fn new_league_ships_free_agents_and_a_scouted_draft_class() {
    let league = generate_league(42);
    let free = free_agents(&league);
    assert!(free.len() >= 40);
    assert!(
        free.iter()
            .all(|p| p.contract.is_none() && p.team_id.is_empty())
    );

    let prospects: Vec<_> = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Prospect)
        .collect();
    assert_eq!(prospects.len(), PROSPECT_CLASS_SIZE);
    assert!(prospects.iter().all(|p| (19..=22).contains(&p.age)));
    assert!(
        prospects
            .iter()
            .all(|p| p.draft_year == Some(league.season))
    );
    // The top prospect has real star upside but is not yet a star.
    let best_potential = prospects.iter().map(|p| p.potential).max().unwrap();
    let best_now = prospects.iter().map(|p| player_overall(p)).max().unwrap();
    assert!(best_potential >= 82, "top potential {best_potential}");
    assert!(best_now < best_potential as u16);
    assert!(prospects.iter().any(|p| p.scouting_fudge != 0));
}

#[test]
fn market_pays_stars_far_more_than_role_players() {
    let league = generate_league(7);
    let mut players: Vec<_> = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Active)
        .collect();
    players.sort_by_key(|p| std::cmp::Reverse(player_overall(p)));
    let star = market_salary(players[0]);
    let role = market_salary(players[players.len() / 2]);
    assert!(star > role * 6, "star {star} vs role {role}");
    assert_eq!(market_salary(players[players.len() - 1]), MIN_SALARY);
    // Asking prices fall as free agency drags on, never below 60%.
    let agent = players[0];
    assert!(ask_salary(agent, 20) < ask_salary(agent, 0));
    assert!(ask_salary(agent, 200) as f64 >= market_salary(agent) as f64 * 0.59);
}

#[test]
fn hard_cap_and_roster_limits_gate_signings() {
    let mut league = generate_league(7);
    let team_id = league.teams[0].id.clone();

    // Fill the cap: the best free agent must not fit.
    let star_fa = free_agents(&league)[0].id.clone();
    let ask = ask_salary(league.players.iter().find(|p| p.id == star_fa).unwrap(), 0);
    let current = payroll(&league, &team_id);
    let mut remaining_pad = SALARY_CAP - current;
    let ids: Vec<String> = league.teams[0].roster.clone();
    for id in ids {
        if remaining_pad < ask {
            break;
        }
        let p = league.players.iter_mut().find(|p| p.id == id).unwrap();
        let contract = p.contract.as_mut().unwrap();
        let bump = (remaining_pad - ask + 1_000).min(MAX_SALARY - contract.salary);
        contract.salary += bump;
        remaining_pad -= bump;
    }
    let room = SALARY_CAP - payroll(&league, &team_id);
    assert!(room < ask, "setup should leave less room than the ask");
    let err = sign_free_agent(&mut league, &team_id, &star_fa, 3).unwrap_err();
    assert!(matches!(err, ContractError::OverCap(_, _)));
    assert_eq!(
        sign_free_agent(&mut league, &team_id, &star_fa, 9),
        Err(ContractError::BadYears)
    );

    // Cut someone: room opens and the signing works; contract is recorded.
    let victim = league.teams[0].roster[0].clone();
    waive_player(&mut league, &team_id, &victim).unwrap();
    let freed = league.players.iter().find(|p| p.id == victim).unwrap();
    assert_eq!(freed.status, PlayerStatus::FreeAgent);
    assert!(freed.contract.is_none());
    let min_fa = free_agents(&league)
        .into_iter()
        .find(|p| ask_salary(p, 0) <= SALARY_CAP - payroll(&league, &team_id))
        .unwrap()
        .id
        .clone();
    let contract = sign_free_agent(&mut league, &team_id, &min_fa, 2).unwrap();
    assert_eq!(contract.years_left, 2);
    assert!(payroll(&league, &team_id) <= SALARY_CAP);
    assert!(league.teams[0].roster.contains(&min_fa));

    // Signed players cannot be signed again.
    assert_eq!(
        sign_free_agent(&mut league, &team_id, &min_fa, 1),
        Err(ContractError::NotFreeAgent)
    );
}

#[test]
fn roster_cap_of_fifteen_is_enforced() {
    let mut league = generate_league(7);
    let team_id = league.teams[3].id.clone();
    // Clear room under the cap by zeroing no one; just sign min-ask guys.
    let mut guard = 0;
    while league.teams[3].roster.len() < ROSTER_MAX && guard < 60 {
        guard += 1;
        let candidate = free_agents(&league)
            .into_iter()
            .rev()
            .find(|p| ask_salary(p, 0) <= SALARY_CAP - payroll(&league, &team_id))
            .map(|p| p.id.clone());
        let Some(candidate) = candidate else { break };
        sign_free_agent(&mut league, &team_id, &candidate, 1).unwrap();
    }
    assert_eq!(league.teams[3].roster.len(), ROSTER_MAX);
    let extra = free_agents(&league).last().unwrap().id.clone();
    assert_eq!(
        sign_free_agent(&mut league, &team_id, &extra, 1),
        Err(ContractError::RosterFull(ROSTER_MAX))
    );
}

#[test]
fn progression_follows_real_age_curves() {
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let mut used = BTreeSet::new();
    let pool = generate_pool(&mut rng, 3000, 1, &mut used);
    let mut by_age: std::collections::BTreeMap<u8, Vec<i16>> = Default::default();
    let mut athletic: Vec<i16> = Vec::new();
    let mut skill: Vec<i16> = Vec::new();
    for player in &pool {
        let mut p = player.clone();
        let before = p.ratings.clone();
        let delta = progress_player(&mut p, &mut rng);
        by_age.entry(player.age).or_default().push(delta);
        if player.age >= 31 {
            athletic.push(
                (p.ratings.perimeter_defense as i16 - before.perimeter_defense as i16)
                    + (p.ratings.interior_defense as i16 - before.interior_defense as i16),
            );
            skill.push(
                (p.ratings.passing as i16 - before.passing as i16)
                    + (p.ratings.three_tendency as i16 - before.three_tendency as i16),
            );
        }
        assert_eq!(p.age, player.age + 1);
    }
    let mean = |ages: &[u8]| -> f64 {
        let values: Vec<i16> = ages
            .iter()
            .filter_map(|a| by_age.get(a))
            .flatten()
            .copied()
            .collect();
        values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64
    };
    assert!(mean(&[19, 20, 21]) > 2.5, "young {}", mean(&[19, 20, 21]));
    assert!(mean(&[22, 23, 24]) > 0.8);
    assert!(
        mean(&[26, 27, 28]).abs() < 1.2,
        "prime {}",
        mean(&[26, 27, 28])
    );
    assert!(mean(&[30, 31, 32]) < -0.7);
    assert!(mean(&[34, 35, 36]) < -2.0, "old {}", mean(&[34, 35, 36]));
    // Athleticism fades faster than skill.
    let avg = |v: &[i16]| v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64;
    assert!(
        avg(&athletic) < avg(&skill) - 0.5,
        "{} vs {}",
        avg(&athletic),
        avg(&skill)
    );
    // The curve itself is monotone around the peak.
    assert!(development_gap(19) > development_gap(23));
    assert!(development_gap(23) > development_gap(27));
    assert!(development_gap(34) > development_gap(30));
}

#[test]
fn high_potential_youngsters_outgrow_low_potential_ones_and_old_players_retire() {
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let mut used = BTreeSet::new();
    let pool = generate_pool(&mut rng, 4000, 1, &mut used);
    let youngsters: Vec<_> = pool.iter().filter(|p| p.age <= 21).collect();
    let (mut hi, mut lo) = (Vec::new(), Vec::new());
    for player in youngsters {
        let mut p = player.clone();
        let delta = progress_player(&mut p, &mut rng) as f64;
        if player.potential as i16 - player_overall(player) as i16 >= 14 {
            hi.push(delta);
        } else {
            lo.push(delta);
        }
    }
    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    assert!(avg(&hi) > avg(&lo), "{} vs {}", avg(&hi), avg(&lo));

    let mut old = pool.iter().find(|p| p.age >= 30).unwrap().clone();
    assert_eq!(retirement_chance(&old), 0.0);
    old.age = 38;
    assert_eq!(retirement_chance(&old), 1.0);
}

#[test]
fn rookie_draft_has_two_rounds_lottery_and_rookie_scale() {
    let mut league = generate_league(7);
    play_season_and_playoffs(&mut league);
    advance_phase(&mut league).expect("draft opens");
    assert_eq!(league.phase, Phase::Draft);
    let draft = league.draft.as_ref().unwrap();
    assert_eq!(draft.order.len(), 64);
    // Each team picks once per round.
    for round in draft.order.chunks(32) {
        assert_eq!(round.iter().collect::<BTreeSet<_>>().len(), 32);
    }
    // Top-4 picks come from non-playoff teams (lottery).
    let playoff_teams: BTreeSet<String> = league.playoffs.as_ref().unwrap().rounds[0]
        .series
        .iter()
        .flat_map(|s| [s.high_team_id.clone(), s.low_team_id.clone()])
        .collect();
    for team in &draft.order[..4] {
        assert!(!playoff_teams.contains(team), "{team} made the playoffs");
    }
    assert_eq!(rookie_order(&league), draft.order, "order is reproducible");

    let first_team = draft.order[0].clone();
    // Wrong team / missing player are rejected.
    let board = available_players(&league);
    let top = board[0].id.clone();
    let other = league
        .teams
        .iter()
        .find(|t| t.id != first_team)
        .unwrap()
        .id
        .clone();
    assert_eq!(
        make_pick(&mut league, &other, &top),
        Err(DraftError::NotOnClock)
    );
    assert_eq!(
        make_pick(&mut league, &first_team, "nope"),
        Err(DraftError::NotAvailable)
    );
    let pick = make_pick(&mut league, &first_team, &top).unwrap();
    assert_eq!(pick.number, 1);
    let rookie = league.players.iter().find(|p| p.id == top).unwrap();
    let contract = rookie.contract.unwrap();
    assert_eq!(contract.years_left, 3);
    assert!(contract.salary > 8_000);
    assert_eq!(rookie.team_id, first_team);

    // AI finishes the rest; later picks earn less.
    let made = run_all(&mut league);
    assert_eq!(made, 63);
    assert!(league.draft.as_ref().unwrap().complete());
    let last = league
        .draft
        .as_ref()
        .unwrap()
        .picks
        .last()
        .unwrap()
        .player_id
        .clone();
    let last_contract = league
        .players
        .iter()
        .find(|p| p.id == last)
        .unwrap()
        .contract
        .unwrap();
    assert!(last_contract.salary < contract.salary / 4);
    assert_eq!(last_contract.years_left, 2);
    assert_eq!(auto_pick(&mut league), Err(DraftError::Complete));
}

#[test]
fn fantasy_draft_builds_cap_legal_rosters_and_pauses_for_humans() {
    let mut league = generate_league_with(11, StartMode::Fantasy);
    assert_eq!(league.phase, Phase::FantasyDraft);
    assert!(league.teams.iter().all(|t| t.roster.is_empty()));
    let total_picks = 32 * FANTASY_ROUNDS;
    assert_eq!(league.draft.as_ref().unwrap().order.len(), total_picks);
    // Snake: round two reverses round one.
    let order = &league.draft.as_ref().unwrap().order;
    assert_eq!(order[0], order[63]);
    assert_eq!(order[31], order[32]);

    // A human on the clock stops the AI.
    let human_team = order[5].clone();
    league
        .teams
        .iter_mut()
        .find(|t| t.id == human_team)
        .unwrap()
        .owner_user_id = Some(uuid::Uuid::new_v4());
    let made = run_ai_until_human(&mut league);
    assert_eq!(made, 5);
    assert_eq!(
        league.draft.as_ref().unwrap().on_the_clock(),
        Some(&human_team)
    );
    // Human picks the best player available; AI continues.
    let best = available_players(&league)[0].id.clone();
    make_pick(&mut league, &human_team, &best).unwrap();
    assert!(run_ai_until_human(&mut league) > 0);

    // Finish everything automatically.
    run_all(&mut league);
    assert_eq!(league.phase, Phase::RegularSeason);
    assert_league_is_legal(&league);
    assert!(
        league
            .teams
            .iter()
            .all(|t| t.roster.len() == FANTASY_ROUNDS)
    );
}

#[test]
fn fantasy_pick_that_blows_the_cap_is_rejected() {
    let mut league = generate_league_with(11, StartMode::Fantasy);
    let team = league.draft.as_ref().unwrap().order[0].clone();
    // Make the team nearly capped out with a fake huge contract on pick one.
    let best = available_players(&league)[0].id.clone();
    make_pick(&mut league, &team, &best).unwrap();
    {
        let p = league.players.iter_mut().find(|p| p.id == best).unwrap();
        p.contract.as_mut().unwrap().salary = SALARY_CAP - 3_000;
    }
    // Skip to the team's next pick (round two, snake: right after).
    let next_team = league.draft.as_ref().unwrap().order[1].clone();
    let a = available_players(&league)[0].id.clone();
    make_pick(&mut league, &next_team, &a).unwrap();
    let mut order_tail = league.draft.as_ref().unwrap().order.clone();
    // Fast-forward AI until the capped team is on the clock again.
    while league.draft.as_ref().unwrap().on_the_clock() != Some(&team) {
        auto_pick(&mut league).unwrap();
        order_tail.pop();
    }
    let star = available_players(&league)
        .into_iter()
        .max_by_key(|p| market_salary(p))
        .unwrap()
        .id
        .clone();
    assert_eq!(
        make_pick(&mut league, &team, &star),
        Err(DraftError::OverCap)
    );
}

fn play_season_and_playoffs(league: &mut League) {
    let ids: Vec<String> = league.schedule.iter().map(|g| g.id.clone()).collect();
    for id in ids {
        simulate_game(league, &id, SimConfig::default());
    }
    assert!(start_playoffs(league));
    let mut guard = 0;
    while champion(league).is_none() && guard < 200 {
        advance_playoff_day(league, SimConfig::default());
        guard += 1;
    }
    assert!(champion(league).is_some());
}

#[test]
fn full_offseason_cycle_rolls_the_league_forward() {
    let mut league = generate_league(7);
    let season = league.season;
    let human = league.teams[0].id.clone();
    league.teams[0].owner_user_id = Some(uuid::Uuid::new_v4());

    // Cannot start the draft mid-season.
    assert!(advance_phase(&mut league).is_err());
    play_season_and_playoffs(&mut league);
    let champ = champion(&league).unwrap();
    let ages_before: Vec<(String, u8)> = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Active)
        .map(|p| (p.id.clone(), p.age))
        .collect();

    advance_phase(&mut league).unwrap(); // draft
    assert_eq!(league.phase, Phase::Draft);
    assert_eq!(league.history.len(), 1);
    assert_eq!(
        league.history[0].champion_team_id.as_deref(),
        Some(champ.as_str())
    );
    assert!(
        league
            .players
            .iter()
            .any(|p| p.history.iter().any(|h| h.season == season))
    );

    advance_phase(&mut league).unwrap(); // draft finishes, players develop
    assert_eq!(league.phase, Phase::Resign);
    let aged = league
        .players
        .iter()
        .filter(|p| p.status != PlayerStatus::Retired)
        .filter(|p| {
            ages_before
                .iter()
                .any(|(id, age)| id == &p.id && p.age == age + 1)
        })
        .count();
    assert!(aged > 300, "most players aged a year ({aged})");
    assert!(
        league
            .players
            .iter()
            .any(|p| p.status == PlayerStatus::Retired),
        "someone retires each summer"
    );
    // Retired players are off rosters.
    for team in &league.teams {
        for id in &team.roster {
            let p = league.players.iter().find(|p| &p.id == id).unwrap();
            assert_ne!(p.status, PlayerStatus::Retired);
        }
    }

    // Re-signing: only in this phase, only expiring players, at market price.
    let expiring = league
        .players
        .iter()
        .find(|p| p.team_id == human && p.contract.is_some_and(|c| c.years_left == 0))
        .map(|p| p.id.clone());
    if let Some(id) = expiring {
        let before = payroll(&league, &human);
        let result = resign_player(&mut league, &human, &id, 3);
        if result.is_ok() {
            assert!(payroll(&league, &human) > before);
        } else {
            assert!(matches!(result, Err(ContractError::OverCap(_, _))));
        }
        // Non-expiring players cannot be "re-signed".
        let healthy = league.teams[0]
            .roster
            .iter()
            .find(|pid| {
                league
                    .players
                    .iter()
                    .any(|p| &p.id == *pid && p.contract.is_some_and(|c| c.years_left > 0))
            })
            .cloned()
            .unwrap();
        assert_eq!(
            resign_player(&mut league, &human, &healthy, 2),
            Err(ContractError::NotExpiring)
        );
    }

    advance_phase(&mut league).unwrap(); // free agency opens
    assert_eq!(league.phase, Phase::FreeAgency);
    assert!(free_agents(&league).len() > 30);
    // Unsigned expiring players left every team they were on.
    for p in &league.players {
        if p.status == PlayerStatus::Active {
            assert!(p.contract.is_some_and(|c| c.years_left > 0));
        }
    }
    sim_free_agency_days(&mut league, 10);
    assert_eq!(league.fa_day, 10);

    advance_phase(&mut league).unwrap(); // next season
    assert_eq!(league.phase, Phase::RegularSeason);
    assert_eq!(league.season, season + 1);
    assert!(league.results.is_empty());
    assert!(league.playoffs.is_none());
    assert_eq!(league.schedule.len(), 1216);
    assert!(league.schedule.iter().all(|g| g.season == season + 1));
    assert_league_is_legal(&league);
    // A fresh draft class is already being scouted for next summer.
    let class = league
        .players
        .iter()
        .filter(|p| p.status == PlayerStatus::Prospect && p.draft_year == Some(season + 1))
        .count();
    assert_eq!(class, PROSPECT_CLASS_SIZE);
    // Standings restart.
    let wins: u16 = basketballman::stats::standings(&league)
        .values()
        .map(|r| r.wins)
        .sum();
    assert_eq!(wins, 0);
}

#[test]
fn enforce_roster_rules_fixes_cap_and_size_problems() {
    let mut league = generate_league(7);
    // Gut one roster and blow another's cap.
    let ids: Vec<String> = league.teams[0].roster.clone();
    let team0 = league.teams[0].id.clone();
    for id in ids.iter().skip(8) {
        waive_player(&mut league, &team0, id).unwrap();
    }
    for id in league.teams[1].roster.clone() {
        let p = league.players.iter_mut().find(|p| p.id == id).unwrap();
        p.contract.as_mut().unwrap().salary = MAX_SALARY / 2;
    }
    enforce_roster_rules(&mut league);
    assert_league_is_legal(&league);
    assert!(league.teams[0].roster.len() >= ROSTER_MIN);
}

#[test]
fn trades_cannot_push_a_team_over_the_hard_cap() {
    use basketballman::trades::{TradeError, validate_offer};
    let mut league = generate_league(7);
    league.teams[0].owner_user_id = Some(uuid::Uuid::new_v4());
    league.teams[1].owner_user_id = Some(uuid::Uuid::new_v4());
    let (a, b) = (league.teams[0].id.clone(), league.teams[1].id.clone());

    // Team A sits at the cap; its cheapest player for B's priciest is illegal.
    let a_ids = league.teams[0].roster.clone();
    let spare = SALARY_CAP - payroll(&league, &a);
    let first = league
        .players
        .iter_mut()
        .find(|p| p.id == a_ids[0])
        .unwrap();
    first.contract.as_mut().unwrap().salary +=
        spare.min(MAX_SALARY - first.contract.unwrap().salary);
    let cheap = a_ids
        .iter()
        .min_by_key(|id| {
            basketballman::contracts::contract_of(&league, id)
                .unwrap()
                .salary
        })
        .unwrap()
        .clone();
    let pricey = league.teams[1]
        .roster
        .iter()
        .max_by_key(|id| {
            basketballman::contracts::contract_of(&league, id)
                .unwrap()
                .salary
        })
        .unwrap()
        .clone();
    {
        let p = league.players.iter_mut().find(|p| p.id == pricey).unwrap();
        p.contract.as_mut().unwrap().salary = 30_000;
    }
    let result = validate_offer(
        &league,
        &a,
        &b,
        std::slice::from_ref(&cheap),
        std::slice::from_ref(&pricey),
    );
    assert!(
        matches!(result, Err(TradeError::OverCap(_))),
        "expected cap failure, got {result:?}"
    );
}
