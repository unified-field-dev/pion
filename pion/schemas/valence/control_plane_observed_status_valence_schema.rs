use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneObservedStatus {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_observed_status",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "High-churn observed state snapshots (heartbeats/runtime status)",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            node_id: { r#type: FieldType::String, required: true },
            source: {
                r#type: FieldType::Enum(&["agent", "scheduler", "reconciler"]),
                required: true,
            },
            health: {
                r#type: FieldType::Enum(&["healthy", "degraded", "unhealthy", "unknown"]),
                required: true,
            },
            observed_json: { r#type: FieldType::Json, required: true },
            observed_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
