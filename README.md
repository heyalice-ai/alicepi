# AlicePi

AlicePi is the Rust runtime for the HeyAlice smart speaker on Raspberry Pi. It is a single Tokio-driven
process that runs an orchestrator, audio tasks, and local/cloud engines in one binary.

## Architecture

The runtime is a single process with internal async tasks, not a set of containerized microservices.

*   **Orchestrator**: Owns the state machine (`Idle`, `Listening`, `Processing`, `Speaking`) and applies
    prioritization rules for button/lid events vs. text/audio input.
*   **Engine**: Local and cloud backends under `src/engine/` for handling inference/session flow.
*   **Tasks**: Tokio tasks for voice input, speech recognition, voice output, and GPIO events.
*   **Watchdog**: Supervises task heartbeats and restarts stalled audio/SR tasks.

## Project Layout

*   `src/main.rs`: CLI entrypoint and runtime bootstrapping.
*   `src/cli.rs`: Client/server subcommands.
*   `src/orchestrator.rs`: State machine + TCP JSON command endpoint.
*   `src/protocol.rs`: Wire types for commands, responses, and status snapshots.
*   `src/engine/`: Local/cloud engine implementations and session handling.
*   `src/tasks/`: Voice input, speech recognition, voice output, and GPIO tasks.
*   `src/watchdog.rs`: Heartbeat monitoring and task restarts.
*   `.cargo/config.toml`: Cross-compilation linker/flags.

## Quick Start

Server:

```
cargo run --bin alicepi -- server --bind 0.0.0.0:7878
```

Client commands:

```
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 ping
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 button
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 text "hello"
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 voice ./samples/utterance.wav
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 audio ./samples/output.wav
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 lid-open
cargo run --bin alicepi -- client --addr 127.0.0.1:7878 lid-close
```

## GPIO

Enable GPIO input support on Raspberry Pi:

```
cargo run --features gpio -- server --gpio-button 17 --gpio-lid 27
```

The lid behavior is feature-flagged for future expansion:

```
cargo run --features lid_control -- server
```

## Cross-Compilation

Build static binaries with musl:

```
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl
```

Build aarch64-unknown-linux-gnu on Fedora (for ALSA, etc.):

```
sudo dnf install gcc-aarch64-linux-gnu glibc-devel.aarch64 alsa-lib-devel.aarch64 pkgconf-pkg-config
SYSROOT="$(aarch64-linux-gnu-gcc -print-sysroot)"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"
export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib64/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_PATH="${PKG_CONFIG_LIBDIR}"
cargo build --release --target aarch64-unknown-linux-gnu
```

Native x86_64 Linux build:

```
cargo build --release
```
