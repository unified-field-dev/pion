use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionAgentHostEnrollment {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_agent_host_enrollment",
        version: "0.1.1",
        database: crate::storage::GLUON_DEFAULT_STORAGE,
        description: "One-time enrollment ticket for joining an agent host to the control plane",

        policies: {
            read: { allow: [AUTHENTICATED] },
            create: { allow: [AUTHENTICATED] },
            update: { allow: [AUTHENTICATED] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            token_hash: { r#type: FieldType::String, required: true },
            expected_agent_host: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            status: {
                r#type: FieldType::Enum(&["pending", "claimed", "revoked"]),
                required: true,
            },
            claimed_node_id: { r#type: FieldType::String },
            setup_wizard_session_id: { r#type: FieldType::String },
            created_at: { r#type: FieldType::DateTime, required: true },
            expires_at: { r#type: FieldType::DateTime, required: true },
            claimed_at: { r#type: FieldType::DateTime },
            revoked_at: { r#type: FieldType::DateTime },
            metadata_json: { r#type: FieldType::Json },
            enrollment_pubkey: { r#type: FieldType::String, required: false, default: "" },
        ]
    }
}
