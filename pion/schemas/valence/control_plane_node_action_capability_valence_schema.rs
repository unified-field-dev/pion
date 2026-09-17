use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneNodeActionCapability {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_node_action_capability",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Capability rows that gate node container lifecycle actions",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            node_id: { r#type: FieldType::String, required: true },
            capability: {
                r#type: FieldType::Enum(&["start", "stop", "restart", "logs", "inspect", "deploy"]),
                required: true,
            },
            enabled: { r#type: FieldType::Boolean, required: true, default: false },
            source: { r#type: FieldType::String },
            reason: { r#type: FieldType::String },
            updated_by: { r#type: FieldType::String },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
