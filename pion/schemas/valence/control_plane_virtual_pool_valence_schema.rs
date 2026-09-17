use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneVirtualPool {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_virtual_pool",
        version: "0.1.2",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Abstract pool grouping that spans cells/providers",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            name: { r#type: FieldType::String, required: true },
            pool_type: {
                r#type: FieldType::String,
                required: true,
                default: "general",
                validations: ["enum:general,photon-worker,boson-worker"],
            },
            description: { r#type: FieldType::String },
            selector_json: { r#type: FieldType::Json, required: true },
            hardware_selector_json: { r#type: FieldType::Json },
            placement_policy_json: { r#type: FieldType::Json },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
