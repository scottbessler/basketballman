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

§I
model: `League` → teams, players, schedule, results, config, seed.
model: `Team` → id, city, name, conference, division, roster.
model: `Player` → id, name, age, position, ratings, team_id.
model: `Game` → id, season, date_index, home_team_id, away_team_id, status.
model: `GameResult` → game_id, home_score, away_score, winner_team_id, team_stats?, player_stats?.
model: `PlayerGameStats` → player_id, team_id, minutes, points, rebounds, assists, steals, blocks, turnovers, fouls, fga, fgm, tpa, tpm, fta, ftm.
view: standings → conference records + sim controls.
view: player → profile + season stat table.
view: game → matchup, final score?, player box score?, sim action?.
svc: `generate_league(seed)` → `League`.
svc: `generate_schedule(league_id, season)` → list `Game`.
svc: `simulate_game(game_id, sim_config)` → `GameResult`.
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

§B
id|date|cause|fix
