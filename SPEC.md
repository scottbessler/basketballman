§G
Basketball legacy management game: NBA-shaped league, generated teams/players, schedule, sim games, store results; Rust SSR backend + light Preact UI.

§C
C1: stack ! Rust web server backend; server-rendered HTML first; Preact only for local interactivity.
C2: default league ! 32 teams, 2 conferences, 16 teams each, real NBA cities plus 2 expansion markets, fake team names.
C3: team/player data ! stable ids, generated names, ratings, roles, contracts? nullable until economy phase.
C4: generation ! seeded RNG ∴ same seed → same league.
C5: schedule ! 76 games/team: 4 games vs each same-conference team (2 home/2 away = 60) + 1 alternating home/away game vs each other-conference team (16).
C6: game result ! final score, winner, team stats, player box score.
C7: persistence ! save league, schedule, played games, results; reload without regenerated ids.
C8: sim v1 options:
  A rating-roll: team strength + pace + variance → plausible final scores; no possession log. Fastest.
  B possession-lite: possessions choose shot/turnover/foul/rebound from team/player ratings → box score. Better feel.
  C player-event: minute allocation + player actions → richer stats. More work.
C9: sim first build ! A; model APIs leave room for B/C.
C10: names ! random fake player names from local first/last pools; no external API required.
C11: fake team names ! avoid real NBA nicknames/logos/marks.
C12: standings page ! show records + sim day/week/month controls.
C13: player season stats ! aggregate from persisted game player stats; visible on team + player pages.
C14: UI ! reuse `../lisports` dense sports table/stat styling, sortable numeric tables, compact nav.
C15: league controls ! reset clears played games/results only; regen creates new generated league.
C16: game page ! schedule game clickable; played game shows box score; unplayed game shows matchup + sim action.
C17: sim engine ! pure function over scheduled game input; engine owns no league loading/persistence.
C18: alt sim engine ! possession-by-possession engine chooses events from ratings, returns same `GameResult` contract.

C19: rotation ! 3 modes per team: `Auto` (coach AI), `Minutes` (starters + minute targets), `Chart` (explicit 12×4-min on-court grid, 5/block).
C20: coach sliders ! foul_caution, starter_load, depth, fatigue_tolerance, + toggles closers/blowout_bench/stagger_stars; apply in all modes (fouls + blowout always; fatigue subs in Auto/Minutes only).
C21: fatigue ! energy 0-100 per player per game; drains on floor by endurance/pace/pressure, recovers on bench; ratings × (0.72+0.28·energy/100).
C22: fouls ! defenders accrue personal fouls; 6 = out; coach benches players in foul trouble by quarter limit [2,3,4,5] shifted by foul_caution; clutch (Q4 close) players stay until 5.
C23: strategy sliders (0-100, 50 neutral) ! offense: pace, three_rate, ball_movement, offensive_glass; defense: pressure, interior_focus, defensive_glass. Each has a cost + benefit.
C24: lineup UX ! SSR form works w/o JS; JS adds touch+mouse drag reorder + paint-drag chart, live 5-per-block validation.
C25: progression ! offseason; age curve peaks 25-27, decline from ~29, steep ≥34; athletic ratings age ~3y earlier than skill ratings; `potential` steers young growth; retirement.
C26: contracts ! `salary` ($K/yr) + `years_left`; hard cap (no signing/trade/draft pick may put payroll > cap); min/max contract; rookie scale.
C27: free agency ! pool of unsigned players; ask = market value; owner signs if cap+roster allow; AI teams sign by value/need; roster 12-15.
C28: draft ! prospects (age 19-22) w/ scouting fuzz; 2 rounds; order = lottery (non-playoff) then record; AI auto-picks, owners pick manually.
C29: offseason phases ! Regular → Playoffs → Draft → Resign → FreeAgency → Regular (next season; new schedule, results archived to history).
C30: new league ! pool of NBA-shaped players (few superstars, many role players); either auto-assigned or fantasy draft (snake, cap-aware: pick must leave room to fill roster at min salary).

§I
model: `League` → teams, players, schedule, results, config, seed.
model: `Team` → id, city, name, conference, division, roster.
model: `Player` → id, name, age, position, ratings, team_id.
model: `Game` → id, season, date_index, home_team_id, away_team_id, status.
model: `GameResult` → game_id, home_score, away_score, winner_team_id, team_stats?, player_stats?.
model: `PlayerGameStats` → player_id, team_id, minutes, points, rebounds, assists, steals, blocks, turnovers, fouls, fga, fgm, tpa, tpm, fta, ftm.
model: `GameSimulationInput` → seed, scheduled `Game`, home/away teams, home/away players, config.
view: standings → conference records + sim controls.
view: player → profile + season stat table.
view: game → matchup, final score?, player box score?, sim action?.
svc: `generate_league(seed)` → `League`.
svc: `generate_schedule(league_id, season)` → list `Game`.
svc: `GameEngine::simulate(input)` → `GameResult`.
svc: `simulate_game(game_id, engine, sim_config)` → loads input, invokes pure engine, persists externally.
repo: save/load league state → durable local store.
repo: reset league state → same teams/players/schedule ids; all games scheduled; results empty.
repo: regenerate league state → new seed/config league; results empty.
web: GET `/` → dashboard.
web: GET `/teams` → team list.
web: GET `/teams/:id` → roster + ratings.
web: GET `/schedule` → schedule + game status.
web: GET `/games/:id` → game detail + box score when played.
web: GET `/standings` → standings + sim day/week/month controls.
web: GET `/players/:id` → player profile + season stats.
web: POST `/games/:id/simulate` → sim one game, persist result, redirect/render.
web: POST `/sim/day` → sim next unplayed date_index, persist, redirect standings.
web: POST `/sim/week` → sim next 7 unplayed date_index values, persist, redirect standings.
web: POST `/sim/month` → sim next 30 unplayed date_index values, persist, redirect standings.
web: POST `/league/reset` → clear results + mark schedule unplayed, persist, redirect standings.
web: POST `/league/regen` → generate new league, persist, redirect standings.
ui: Preact islands → filters, table sorting, simulate buttons/progress.

§V
V1: ∀ persisted entity → stable unique id; reload preserves ids.
V2: default league → exactly 32 teams, 2 conferences, 16 teams/conference.
V3: default team city set includes real NBA city/market set; nicknames fake ∧ ≠ NBA nicknames.
V4: ∀ team → roster size ≥ 12 at generation.
V5: ∀ player → name non-empty ∧ generated from local pools.
V6: same seed + same config → same teams, players, schedule.
V7: schedule generation → each game has distinct id ∧ valid home/away teams ∧ home_team_id ≠ away_team_id.
V8: default regular season → each team has 76 games: 60 same-conference + 16 other-conference.
V9: unplayed game → no `GameResult`.
V10: simulated game → exactly one `GameResult` ∧ game status played.
V11: result winner_team_id ∈ {home_team_id, away_team_id}.
V12: sim v1 score → home_score > 0 ∧ away_score > 0.
V13: web mutating actions → persist before response.
V14: SSR route works without JS; Preact enhances only.
V15: simulated game → player_stats exists ∧ covers both active rosters.
V16: player season stats = sum(player_stats for persisted results).
V17: standings record = wins/losses from persisted results.
V18: sim range action → sims only scheduled games in next requested unplayed date_index window.
V19: reset action → same team/player/game ids ∧ results empty ∧ all games scheduled.
V20: regen action → fresh generated league ∧ results empty ∧ valid default shape.
V21: game detail route → played game shows player box score; unplayed game shows no result.
V22: ∀ schedule date_index → each team appears ≤ 1 game.
V23: game engine simulate → pure over `GameSimulationInput`; no league load/save/mutation.
V24: all game engines → same `GameResult` contract ∧ valid winner/player_stats/positive scores.
V25: possession engine → team_stats.possessions > 0 ∧ player scoring sums to team scores.

§T
id|status|task|cites
T1|x|create Rust web project skeleton + SSR layout|C1,I.web,V14
T2|x|define domain models + ids for league/team/player/game/result|C3,I.model,V1,V9,V10,V11
T3|x|define default NBA-shaped city/conference/division config with fake names|C2,C11,V2,V3
T4|x|build seeded player/team generator with local name pools + ratings|C4,C10,I.svc,V4,V5,V6
T5|x|build NBA-shaped 82-game schedule generator|C5,I.svc,V6,V7,V8
T6|x|add durable local repository for league state + reload|C7,I.repo,V1,V9,V10
T7|x|implement sim v1 rating-roll engine|C8,C9,I.svc,V10,V11,V12
T8|x|wire simulate-one-game web action + result persistence|I.web,V10,V13,V14
T9|x|render dashboard, teams, team detail, schedule pages|I.web,I.ui,V14
T10|x|add focused tests for generation, schedule, sim, persistence invariants|V1,V2,V3,V4,V5,V6,V7,V8,V9,V10,V11,V12,V13
T11|x|expand league to 32 teams + 76-game schedule|C2,C5,V2,V3,V6,V7,V8
T12|x|add player game stats + season aggregates|C6,C13,I.model,V10,V15,V16
T13|x|add standings + sim day/week/month actions|C12,I.web,V13,V17,V18
T14|x|add player pages + team season stat tables|C13,I.web,V14,V16
T15|x|reuse lisports table/stat styling + sortable tables|C14,I.ui,V14
T16|x|add reset + regen league controls|C15,I.web,I.repo,V13,V19,V20
T17|x|add game detail page + clickable schedule games|C16,I.web,V14,V21
T18|x|test reset/regen/box-score invariants|V13,V19,V20,V21
T19|x|extract pure game engine interface + rating-roll engine|C17,I.model,I.svc,V23,V24
T20|x|add possession-by-possession engine|C18,I.svc,V24,V25
T21|x|wire web simulation through engine interface + tests|I.web,V10,V13,V23,V24,V25
T22|x|endurance rating + in-game fatigue energy/skill penalty|C21
T23|x|personal fouls + foul-out|C22
T24|x|team strategy sliders offense/defense in sim|C23
T25|x|coach: auto/minutes/chart substitution engine + coach sliders + auto chart builder|C19,C20,C22
T26|x|lineup editor page: depth drag/drop, paint-drag chart, sliders; mobile/touch|C24
T27|.|NBA-shaped player pool generator (tiers, archetypes, ages, potential)|C30
T28|.|contracts + hard cap + payroll UI; trades obey cap|C26
T29|.|free agency: pool, sign, waive, AI signings, roster limits|C27
T30|.|age-based progression + retirement + player history|C25
T31|.|prospect classes + scouting fuzz + lottery + rookie draft|C28
T32|.|offseason phase flow + season rollover|C29
T33|.|new-league flow w/ fantasy draft (cap-aware)|C30
T34|.|tests + docs for all of the above|V26..

§B
id|date|cause|fix
B1|2026-07-09|date_index assigned by raw 16-game chunks, not daily matchings|V22
