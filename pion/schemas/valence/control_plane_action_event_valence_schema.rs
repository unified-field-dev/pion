use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneActionEvent {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_action_event",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Audit/event records for node container lifecycle action attempts",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            node_id: { r#type: FieldType::String, required: true },
            container_ref: { r#type: FieldType::String, required: true },
            action: {
                r#type: FieldType::Enum(&["start", "stop", "restart", "logs", "inspect", "deploy"]),
                required: true,
            },
            status: {
                r#type: FieldType::Enum(&["success", "denied", "failed"]),
                required: true,
            },
            message: { r#type: FieldType::String, required: true },
            payload_json: { r#type: FieldType::Json },
            observed_at: { r#type: FieldType::DateTime, required: true },
            created_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
