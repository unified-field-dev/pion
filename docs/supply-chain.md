# Supply chain policy

Pion pins third-party crates through `Cargo.lock` and enforces dependency policy with
[`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) (`deny.toml`).

## What CI checks

The `deny` job in CI runs `cargo deny check` on every push and pull request to `main`. That covers:

- RustSec advisories (with documented ignores in `deny.toml`)
- Allowed license set
- Allowed crate sources (crates.io plus the Git remotes listed below)

## Unified Field dependencies

Workspace `[workspace.dependencies]` pins upstream cores and Parton as git
`branch = "main"` from `unified-field-dev`.

| Crate | Source |
|-------|--------|
| `uf-valence` / `uf-valence-codegen` | Git `unified-field-dev/valence` `branch = "main"` |
| `uf-spectra` / `uf-spectra-core` / `spectra-macros` | Git `unified-field-dev/Spectra` `branch = "main"` |
| `uf-photon` | Git `unified-field-dev/photon` `branch = "main"` |
| `parton` | Git `unified-field-dev/parton` `branch = "main"` |
| `chronon-core` / `chronon-executor` | Git `unified-field-dev/chronon` `branch = "main"` (optional `chronon` feature) |
| `chronon-coordinator` | Git `unified-field-dev/chronon-coordinator` `branch = "main"` |
| `chronon-coordinator-macros` | Git `unified-field-dev/chronon-coordinator-macros` `branch = "main"` |
| `chronon-valence-identity` | Git `unified-field-dev/chronon-valence-identity` `branch = "main"` |

Those Git remotes are listed in [`deny.toml`](../deny.toml) `[sources].allow-git`.

## Rules

1. **Prefer crates.io** for all new third-party dependencies.
2. **New Git dependencies** require:
   - An entry in [`deny.toml`](../deny.toml) `[sources].allow-git`
   - A short note in this file (why Git, which rev, migration plan)
3. **Ignored advisories** in [`deny.toml`](../deny.toml) must cite the affected crate and a
   removal trigger (for example, "remove once crate X ships a patched release").

## Advisory ignores

None currently. If an advisory ignore is added, it must include a `reason` in `deny.toml` and
a corresponding entry here.

## Verification

```bash
cargo install cargo-deny --locked
cargo deny check
```

See [`docs/VERIFICATION.md`](VERIFICATION.md) for the full verification command block.
