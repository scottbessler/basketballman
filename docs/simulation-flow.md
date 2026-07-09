# Simulation Flow

```mermaid
flowchart LR
  subgraph Inputs
    A["League seed"]
    B["Teams + rosters"]
    C["Player ratings"]
    D["Schedule game_id"]
    E["SimConfig: home_advantage, variance"]
  end

  subgraph Process
    F["Load scheduled Game"]
    G["Find home + away Team"]
    H["Compute team_rating from roster ratings"]
    I["Seed game RNG from league seed + game_id"]
    J["Roll pace + score variance"]
    K["Calculate home_score + away_score"]
    L["Distribute points/minutes/stats across players"]
    M["Set winner + mark game Played"]
  end

  subgraph Outputs
    N["GameResult"]
    O["TeamStats"]
    P["PlayerGameStats box score"]
    Q["Persisted League.results"]
    R["Standings + player season aggregates"]
  end

  A --> I
  B --> G
  C --> H
  D --> F
  E --> J
  F --> G --> H --> K
  I --> J --> K --> L --> M
  M --> N
  K --> O
  L --> P
  N --> Q
  O --> Q
  P --> Q
  Q --> R
```

## Summary

Inputs: persisted league, target `game_id`, roster ratings, and `SimConfig`.

Process: find matchup, derive team strength, create deterministic game RNG, roll score, generate player lines, mark game played.

Outputs: final score, winner, team stats, player box score, persisted result, standings, and season stat aggregates.
