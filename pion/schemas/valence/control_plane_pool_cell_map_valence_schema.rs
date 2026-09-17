use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlanePoolCellMap {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_pool_cell_map",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Mapping between virtual pools and cells",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            virtual_pool_id: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            weight: { r#type: FieldType::Integer, required: true, default: 1 },
            enabled: { r#type: FieldType::Boolean, required: true, default: true },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
