# pion-spectra-topics

Stable Spectra topic-name constants for the Pion control plane. Depend on this crate when
dashboards, NDJSON exporters or subscribers need the event and metric names that `pion` emits,
without pulling in the full `pion` runtime.

```toml
[dependencies]
pion-spectra-topics = "0.1"
```

```rust
use pion_spectra_topics::event;

assert_eq!(event::HEARTBEAT_LOG, "pion_heartbeat_log");
```

Wire names follow `spectra.event.{table}` and `spectra.metric.{name}`. A test in this crate
keeps the constants in sync with the schemas registered by `pion`.

See the [repository README](https://github.com/unified-field-dev/pion#readme) for the control
plane itself.
