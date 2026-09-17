use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneEndpoints {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_endpoints",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Computed service endpoints for discovery and health reporting",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            service_id: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String },
            endpoints_json: { r#type: FieldType::Json, required: true },
            healthy_count: { r#type: FieldType::Integer, required: true, default: 0 },
            total_count: { r#type: FieldType::Integer, required: true, default: 0 },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
