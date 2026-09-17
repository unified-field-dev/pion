use valence::prelude::*;
use valence::privacy_policies::common::SYSTEM_ONLY;

valence_schema! {
    PionNodeActionResult {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_node_action_result",
        version: "0.1.0",
        database: crate::storage::GLUON_DEFAULT_STORAGE,
        description: "Execution result reported by agent for a node action command",

        policies: {
            read:   { allow: [SYSTEM_ONLY] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            command_id: { r#type: FieldType::String, required: true },
            node_id: { r#type: FieldType::String, required: true },
            attempt: { r#type: FieldType::Integer, required: true },
            success: { r#type: FieldType::Boolean, required: true },
            stdout: { r#type: FieldType::String },
            stderr: { r#type: FieldType::String },
            error_summary: { r#type: FieldType::String },
            payload_json: { r#type: FieldType::Json },
            observed_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
