use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneNode {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_node",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Node inventory tracked per control-plane cell",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            hostname: { r#type: FieldType::String, required: true },
            connection_mode: {
                r#type: FieldType::Enum(&["agent", "ssh"]),
                required: true,
            },
            status: {
                r#type: FieldType::Enum(&["online", "offline", "draining", "failed"]),
                required: true,
            },
            failure_domain: { r#type: FieldType::String },
            capabilities_json: { r#type: FieldType::Json },
            labels_json: { r#type: FieldType::Json },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
