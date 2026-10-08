§G
Game sim engine spec: pure `GameSimulationInput` → `GameResult`; possession engine models each team possession-by-possession.

§C
C1: engine ! pure; no league load, save, result insert, game status mutation.
C2: caller owns persistence + idempotency; engine owns only result generation.
C3: same `seed` + `game.id` + engine `name()` + input ratings/config → same result.
C4: all engines ! return same `GameResult` shape.
C5: possession engine ! produce player box score from individual possessions.
C6: current default web engine = `PossessionEngine`.
C7: rating-roll engine remains available for fast/legacy sim.

§I
trait: `GameEngine` → `name()`, `simulate(input)`.
fn: `GameEngine::name()` → static engine id string.
fn: `GameEngine::simulate(&GameSimulationInput)` → `GameResult`.
model: `GameSimulationInput` → seed, scheduled `Game`, home_team, away_team, home_players, away_players, `SimConfig`.
model: `SimConfig` → home_advantage, variance.
engine: `RatingRollEngine` → team-rating score roll + distributed player stats.
engine: `PossessionEngine` → possession loop + event outcomes + player stats.
wrapper: `simulate_game_with_engine(league, game_id, engine, config)` → build input, call engine, mark played, persist result in caller-owned league state.
wrapper: `simulate_game(league, game_id, config)` → default `PossessionEngine`.

§Input
I1: `seed` ! league seed.
I2: `game` ! scheduled `Game`; supplies id, season, home_team_id, away_team_id.
I3: `home_team`, `away_team` ! resolved by caller/input builder.
I4: `home_players`, `away_players` ! roster players in team roster order.
I5: `config.home_advantage` ! applied to home possession shot quality.
I6: `config.variance` ! used by rating-roll engine; currently unused by possession engine.

§Process.Possession
P1: seed RNG from `seed`, `game.id`, engine name `"possession"`.
P2: roll total possessions: 96..=106.
P3: create empty player stat lines for both active rosters.
P4: for each possession, simulate home offense then away offense.
P5: choose shooter by weighted offense + shooting + playmaking.
P6: compute defense pressure from opposing average defense.
P7: event order:
  a. foul roll < 8 → 2 or 3 free throws.
  b. turnover roll < threshold → turnover.
  c. shot attempt → 2PA or 3PA.
P8: 3PA chance increases with shooter shooting rating.
P9: make threshold uses shooter offense/shooting, opposing defense, home advantage.
P10: made FG → add 2 or 3 points; maybe credit assist.
P11: missed FG → credit rebound to offensive roster by rebounding weight.
P12: tied final score → add 1 free throw point to highest-minute player on random side.
P13: merge home + away stat lines.
P14: produce `GameResult` via shared score/result builder.

§Output
O1: `GameResult.game_id` = input `game.id`.
O2: `home_score`, `away_score` = sum of possession events.
O3: `winner_team_id` ∈ `{home_team_id, away_team_id}`.
O4: `team_stats.possessions` = possession count.
O5: `team_stats.offensive_rating` = home roster average rating.
O6: `team_stats.defensive_rating` = away roster average rating.
O7: `player_stats` ! present; one row per active roster player.
O8: player scoring by team sums exactly to team score.

§Fatigue+Rotation
F1: each player has `energy` 0-100 (start 100). On floor: −(6.4−0.04·endurance)/min × pace/pressure load. Bench: +(2.4+0.015·endurance)/min. Quarter break +6, halftime +22.
F2: effective ratings = ratings × (0.72+0.28·energy/100); shooting/FT percentages feel half the penalty; endurance itself never degrades.
F3: defenders accrue personal fouls (shooting fouls + loose fouls, ~21/team/game); 6 = out, never re-enters.
F4: coach (`src/coach.rs`) picks 5 each possession: modes Auto | Minutes | Chart. Tip-off and 3rd-quarter start use starters (Chart: block 0).
F5: foul trouble: sit at [2,3,4,5] fouls by quarter, shifted ±1 by `foul_caution`; clutch (last 6 min, ≤10 pts, `closers`) limit 5 and best five stay (energy ≥25).
F6: blowout (`blowout_bench`): from Q3 on, |lead| ≥ 10+1.25·minutes_left → players outside the top five play.
F7: dynamic subs score = quality×fatigue + minutes owed vs pro-rata target + stint hysteresis; resting flag below `70−0.4·fatigue_tolerance` energy until +20 recovered; min stint 120 s, min rest 90 s; ≥1 ball handler + ≥1 big on floor; `stagger_stars` keeps one of the top two on floor.
F8: Chart mode = 12×4-min blocks, exactly 5 distinct roster ids each; invalid chart falls back to Minutes/Auto. Chart honors plan except fouls/blowout/clutch.
F9: strategy sliders (0-100, 50 neutral): pace (±7 possessions, transition, drain), three_rate (±10% 3PA), ball_movement (usage spread, assists, +quality, +turnovers), offensive_glass (+OREB, +opp fast breaks), pressure (+steals,+turnovers,+fouls,+drain), interior_focus (+interior contest, −perimeter), defensive_glass (+DREB, −own fast breaks).
F10: fast break after steal/DREB/live TO: 2-pt only, +12 make threshold, rarely turns over.

§V
V1: engine simulate ⊥ load/save/mutate league.
V2: same input + same engine → same `GameResult`.
V3: all engine outputs → positive scores ∧ winner in matchup.
V4: all engine outputs → `player_stats` present ∧ covers home+away active rosters.
V5: possession output → `team_stats.possessions > 0`.
V6: possession output → player point sums equal `home_score`/`away_score`.
V7: wrapper only place allowed to mutate `League.results` or game status.
V8: engine `name()` participates in RNG seed ∴ engines can produce distinct deterministic outputs.
V9: no player exceeds 6 fouls or 48 minutes; team minutes ≈ 240.
V10: sim output deterministic for same seed+game+rosters+strategy.

§T
id|status|task|cites
T1|x|extract pure `GameEngine` trait|C1,C4,I.trait,V1,V2,V3,V4
T2|x|move rating-roll behind trait|C7,I.engine,V2,V3,V4
T3|x|add possession engine|C5,C6,I.engine,P1,P2,P3,P4,P5,P6,P7,P8,P9,P10,P11,P12,P13,P14,V5,V6
T4|x|wire wrapper to default possession engine|C2,C6,I.wrapper,V7
T5|x|test pure engines + possession contract|V1,V2,V3,V4,V5,V6,V8

§B
id|date|cause|fix
