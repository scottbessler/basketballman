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

§V
V1: engine simulate ⊥ load/save/mutate league.
V2: same input + same engine → same `GameResult`.
V3: all engine outputs → positive scores ∧ winner in matchup.
V4: all engine outputs → `player_stats` present ∧ covers home+away active rosters.
V5: possession output → `team_stats.possessions > 0`.
V6: possession output → player point sums equal `home_score`/`away_score`.
V7: wrapper only place allowed to mutate `League.results` or game status.
V8: engine `name()` participates in RNG seed ∴ engines can produce distinct deterministic outputs.

§T
id|status|task|cites
T1|x|extract pure `GameEngine` trait|C1,C4,I.trait,V1,V2,V3,V4
T2|x|move rating-roll behind trait|C7,I.engine,V2,V3,V4
T3|x|add possession engine|C5,C6,I.engine,P1,P2,P3,P4,P5,P6,P7,P8,P9,P10,P11,P12,P13,P14,V5,V6
T4|x|wire wrapper to default possession engine|C2,C6,I.wrapper,V7
T5|x|test pure engines + possession contract|V1,V2,V3,V4,V5,V6,V8

§B
id|date|cause|fix
