use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneCell {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_cell",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "A control-plane cell boundary (region/provider/homelab)",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            name: { r#type: FieldType::String, required: true },
            provider: { r#type: FieldType::String, required: true },
            region: { r#type: FieldType::String, required: true },
            mode: {
                r#type: FieldType::Enum(&["local", "cloud", "edge"]),
                required: true,
            },
            status: {
                r#type: FieldType::Enum(&["active", "draining", "offline", "failed"]),
                required: true,
            },
            desired_state_json: { r#type: FieldType::Json },
            metadata_json: { r#type: FieldType::Json },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
