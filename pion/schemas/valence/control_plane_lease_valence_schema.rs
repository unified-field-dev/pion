use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneLease {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_lease",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Lease/leadership records scoped by cell_id",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            lease_name: { r#type: FieldType::String, required: true },
            leased_by: { r#type: FieldType::String, required: true },
            lease_until: { r#type: FieldType::DateTime, required: true },
            status: {
                r#type: FieldType::Enum(&["active", "expired"]),
                required: true,
            },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
