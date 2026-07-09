§G
Basketball legacy management game: NBA-shaped league, generated teams/players, schedule, sim games, store results; Rust SSR backend + light Preact UI.

§C
C1: stack ! Rust web server backend; server-rendered HTML first; Preact only for local interactivity.
C2: default league ! 30 teams, 2 conferences, 6 divisions, real NBA cities, fake team names.
C3: team/player data ! stable ids, generated names, ratings, roles, contracts? nullable until economy phase.
C4: generation ! seeded RNG ∴ same seed → same league.
C5: schedule ! NBA-shaped regular season; default 82 games/team; no playoffs in first slice.
C6: game result ! final score, winner, team stats, player box score? optional in v1.
C7: persistence ! save league, schedule, played games, results; reload without regenerated ids.
C8: sim v1 options:
  A rating-roll: team strength + pace + variance → plausible final scores; no possession log. Fastest.
  B possession-lite: possessions choose shot/turnover/foul/rebound from team/player ratings → box score. Better feel.
  C player-event: minute allocation + player actions → richer stats. More work.
C9: sim first build ! A; model APIs leave room for B/C.
C10: names ! random fake player names from local first/last pools; no external API required.
C11: fake team names ! avoid real NBA nicknames/logos/marks.

§I
model: `League` → teams, players, schedule, results, config, seed.
model: `Team` → id, city, name, conference, division, roster.
model: `Player` → id, name, age, position, ratings, team_id.
model: `Game` → id, season, date_index, home_team_id, away_team_id, status.
model: `GameResult` → game_id, home_score, away_score, winner_team_id, team_stats?, player_stats?.
svc: `generate_league(seed)` → `League`.
svc: `generate_schedule(league_id, season)` → list `Game`.
svc: `simulate_game(game_id, sim_config)` → `GameResult`.
repo: save/load league state → durable local store.
web: GET `/` → dashboard.
web: GET `/teams` → team list.
web: GET `/teams/:id` → roster + ratings.
web: GET `/schedule` → schedule + game status.
web: POST `/games/:id/simulate` → sim one game, persist result, redirect/render.
ui: Preact islands → filters, table sorting, simulate buttons/progress.

§V
V1: ∀ persisted entity → stable unique id; reload preserves ids.
V2: default league → exactly 30 teams, 2 conferences, 6 divisions.
V3: default team city set = real NBA city/market set; nicknames fake ∧ ≠ NBA nicknames.
V4: ∀ team → roster size ≥ 12 at generation.
V5: ∀ player → name non-empty ∧ generated from local pools.
V6: same seed + same config → same teams, players, schedule.
V7: schedule generation → each game has distinct id ∧ valid home/away teams ∧ home_team_id ≠ away_team_id.
V8: default regular season → each team has 82 games.
V9: unplayed game → no `GameResult`.
V10: simulated game → exactly one `GameResult` ∧ game status played.
V11: result winner_team_id ∈ {home_team_id, away_team_id}.
V12: sim v1 score → home_score > 0 ∧ away_score > 0.
V13: web mutating actions → persist before response.
V14: SSR route works without JS; Preact enhances only.

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

§B
id|date|cause|fix
