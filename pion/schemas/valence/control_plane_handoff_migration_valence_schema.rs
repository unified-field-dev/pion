use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneHandoffMigration {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_handoff_migration",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Control-plane authority migration session (overlap window, one-time token binding)",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            authority_id: { r#type: FieldType::String, required: true },
            phase: {
                r#type: FieldType::Enum(&[
                    "preparing",
                    "overlap",
                    "new_only",
                    "completed",
                    "failed",
                    "revoked",
                ]),
                required: true,
            },
            expected_jti_hash: { r#type: FieldType::String, required: true },
            token_consumed: { r#type: FieldType::Boolean, required: true, default: false },
            overlap_not_before: { r#type: FieldType::DateTime },
            overlap_not_after: { r#type: FieldType::DateTime },
            status_message: { r#type: FieldType::String },
            last_error: { r#type: FieldType::String },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
            finalized_at: { r#type: FieldType::DateTime },
        ]
    }
}
