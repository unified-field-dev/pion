use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionContainerObservation {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_container_observation",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Per-container observation upserted from agent heartbeats",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            node_id: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            container_id: { r#type: FieldType::String, required: true },
            container_name: { r#type: FieldType::String, required: true },
            instance_id: { r#type: FieldType::String, required: false, default: "" },
            state: {
                r#type: FieldType::Enum(&[
                    "created", "running", "restarting", "paused",
                    "exited", "dead", "removing"
                ]),
                required: true,
            },
            exit_code: { r#type: FieldType::Integer, required: false },
            started_at: { r#type: FieldType::DateTime, required: false },
            finished_at: { r#type: FieldType::DateTime, required: false },
            health: {
                r#type: FieldType::Enum(&[
                    "starting", "healthy", "unhealthy", "no_check", "unknown"
                ]),
                required: true,
            },
            restart_count: { r#type: FieldType::Integer, required: true, default: 0 },
            first_observed_at: { r#type: FieldType::DateTime, required: true },
            last_observed_at: { r#type: FieldType::DateTime, required: true },
            ingested_at: { r#type: FieldType::DateTime, required: true },
            probe_error: { r#type: FieldType::String, required: false, default: "" },
        ]
    }
}
