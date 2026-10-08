# basketballman

A basketball legacy management game: a generated NBA-shaped league with
schedules, simulated games, box scores, standings, trades and playoffs.
Rust (axum) with server-rendered HTML.

Features: possession-level sim with fatigue, fouls and a coach AI; lineup
strategy sliders + auto/minutes/chart rotations; contracts under a hard cap, free
agency, a rookie draft, age-based progression and a fantasy-draft league start.

Live at https://basketballman.fly.dev.

## Development

```sh
./scripts/setup.sh   # git hooks, crates, warm build
./dev.sh             # http://localhost:3000, restarts on change, passkeys off
mise run check       # everything CI runs
```

See [AGENTS.md](AGENTS.md) for commands and conventions, and
[SPEC.md](SPEC.md) / [SIM.SPEC.md](SIM.SPEC.md) for game design.

## Configuration

| Env | Default | Purpose |
| --- | --- | --- |
| `PORT` | `3000` | Listen port |
| `DATA_PATH` | `data` | Directory for `league.json` and user store |
| `RP_ID` / `RP_ORIGIN` | `localhost` / `http://localhost:3000` | WebAuthn relying party |
| `SESSION_SECRET` | dev fallback in debug builds | Cookie signing key (64+ bytes) |
| `PASSKEY_DISABLED` | unset | `1` skips passkey auth for local use |
