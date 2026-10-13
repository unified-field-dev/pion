# pion-server

Slim headless server for the Pion cell control plane. It serves the Parton agent ingest routes
(`/api/parton/*`) on Valence SQLite or Hybrid storage, plus health, readiness and Prometheus
endpoints.

## Run

```bash
PARTON_SHARED_TOKEN=dev-shared-token cargo run -p pion-server
```

In a second shell:

```bash
curl -s localhost:3000/health
curl -s localhost:3000/cell/readiness   # {"ready":true}
```

The server binds `127.0.0.1:3000` by default (set `PION_BIND` to change it) and refuses to
start without `PARTON_SHARED_TOKEN` unless `PION_ALLOW_INSECURE=1` is set for local testing.

Routes, metrics and every environment variable are listed in the crate documentation:
`cargo doc -p pion-server --open`. For the library this binary wraps, see the
[repository README](https://github.com/unified-field-dev/pion#readme).
