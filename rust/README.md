# AlicePi Rust Runtime

This folder contains a Rust rewrite of the AlicePi runtime with a single binary that can run
in server mode or client mode.

## Quick start

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

## Cross-compilation

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
