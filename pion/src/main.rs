//! Standalone `pion` binary entrypoint (split control-plane image / operator process).
//!
//! The library crate holds all Valence and HTTP-adjacent logic; this binary is reserved for a thin
//! process wrapper once routing and env config are finalized. Until then it exits with a clear
//! message so operators do not mistake an empty stub for a listening server.
//!
//! # See also
//!
//! - [`README.md`](https://github.com/unified-field-dev/pion/blob/main/pion/README.md) for intended deployment shapes.
//! - The headless [`pion-server`](https://github.com/unified-field-dev/pion/blob/main/pion-server) binary for ingest.

// Stub exits immediately; stdout/stderr is the operator-facing message until wiring lands.
#![allow(clippy::print_stderr)]

fn main() {
    eprintln!(
        "pion: standalone binary wiring is not enabled yet; use the `pion-server` headless image \
         or embed `pion` as a library in your own server binary."
    );
    std::process::exit(1);
}
