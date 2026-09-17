use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneService {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_service",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Service discovery record within a cell or globally",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            name: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String },
            selector_json: { r#type: FieldType::Json, required: true },
            ports_json: { r#type: FieldType::Json, required: true },
            discovery_mode: {
                r#type: FieldType::Enum(&["cell", "global"]),
                required: true,
            },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
