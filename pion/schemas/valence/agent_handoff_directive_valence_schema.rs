use valence::prelude::*;
use valence::privacy_policies::common::SYSTEM_ONLY;

valence_schema! {
    PionAgentHandoffDirective {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_agent_handoff_directive",
        version: "0.1.1",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Signed re-enroll directive for Parton agents (CP handoff stack)",

        policies: {
            read: { allow: [SYSTEM_ONLY] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            target_node_id: { r#type: FieldType::String, required: true },
            new_cp_url: { r#type: FieldType::String, required: true },
            new_authority_id: { r#type: FieldType::String, required: true },
            new_authority_verify_key: { r#type: FieldType::String, required: true },
            new_token_ciphertext: { r#type: FieldType::String, required: true },
            new_token_sha256_hex: { r#type: FieldType::String, required: true },
            not_before: { r#type: FieldType::DateTime, required: true },
            not_after: { r#type: FieldType::DateTime, required: true },
            issued_by_authority_id: { r#type: FieldType::String, required: true },
            issued_at: { r#type: FieldType::DateTime, required: true },
            signature: { r#type: FieldType::String, required: true },
            status: {
                r#type: FieldType::Enum(&["pending", "delivered", "acknowledged", "expired", "failed"]),
                required: true,
            },
            delivered_at: { r#type: FieldType::DateTime },
            acknowledged_at: { r#type: FieldType::DateTime },
            failure_reason: { r#type: FieldType::String },
            failed_at: { r#type: FieldType::DateTime },
        ]
    }
}
