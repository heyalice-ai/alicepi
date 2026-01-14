# Rust Runtime Notes

## Purpose
This `rust/` folder is a Rust rewrite of the AlicePi runtime, using a single Tokio-based binary (`alicepi`) that can run in server or client mode. It replaces the prior Python microservice orchestration with an internal, message-driven runtime and supports cross-compilation.

## Key Behaviors
- **Server mode** hosts a TCP JSON command endpoint and orchestrates state (`Idle`, `Listening`, `Processing`, `Speaking`).
- **Client mode** sends JSON commands (ping, status, text, voice file injection, audio playback, button/lid events).
- **Button semantics**: pressing a button unmutes mic, starts listening, and cancels current output/LLM response; VAD silence re-mutes.
- **Lid semantics**: feature-flagged (lid_control). Default assumes lid open unless changed via client; GPIO lid support is optional.
- **Watchdog**: voice_input and speech_rec tasks are supervised; if heartbeats stop, tasks are restarted.

## Layout
- `src/main.rs`: CLI entrypoint, client/server modes.
- `src/cli.rs`: CLI subcommands and help strings.
- `src/orchestrator.rs`: state machine + TCP server + status logging.
- `src/protocol.rs`: wire types for client commands, server replies, and status snapshots.
- `src/tasks/`: voice_input, speech_rec, voice_output, gpio modules.
- `.cargo/config.toml`: cross-compilation linker/flags.

## Cross-Compilation
- Targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static), and native Linux.
- `musl-cross-make` is used to produce `aarch64-linux-musl-gcc` on Fedora. Ensure `$HOME/musl/bin` is on PATH and `linker = "aarch64-linux-musl-gcc"` for the musl target.
- Zig attempts were dropped; use musl toolchain or GNU target for non-static.

## Status & Observability
- `client status` returns `state`, `mic_muted`, `lid_open`.
- Server logs any state, mic, or lid changes to console.

## TODO / Known Gaps
- Audio I/O, VAD, and speech recognition are stubbed; integration is pending.
- Voice output is a placeholder log; needs ALSA playback implementation.
