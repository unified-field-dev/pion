use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneHandoff {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_handoff",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Authority handoff lifecycle for bootstrap operations",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            bootstrap_id: { r#type: FieldType::String, required: true },
            authority_mode: {
                r#type: FieldType::Enum(&["local", "transitioning", "remote"]),
                required: true,
            },
            handoff_state: {
                r#type: FieldType::Enum(&[
                    "pre_bootstrap",
                    "bootstrap_in_progress",
                    "remote_ready",
                    "handoff_completed",
                    "handoff_failed",
                ]),
                required: true,
            },
            remote_ready: { r#type: FieldType::Boolean, required: true, default: false },
            remote_status_url: { r#type: FieldType::String },
            status_message: { r#type: FieldType::String },
            last_error: { r#type: FieldType::String },
            updated_at: { r#type: FieldType::DateTime, required: true },
            completed_at: { r#type: FieldType::DateTime },
        ]
    }
}
