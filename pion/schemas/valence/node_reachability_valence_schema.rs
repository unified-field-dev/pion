use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionNodeReachability {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_node_reachability",
        version: "0.1.0",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Per-node dial targets: CP-observed peer IP and operator overrides for CP vs runtime container reachability",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            node_id: { r#type: FieldType::Record("pion_control_plane_node"), required: true },
            peer_ip: { r#type: FieldType::String, required: false, default: "" },
            cp_connect_host: { r#type: FieldType::String, required: false, default: "" },
            cp_connect_source: {
                r#type: FieldType::Enum(&["peer_ip", "operator"]),
                required: true,
                default: "peer_ip",
            },
            runtime_connect_host: { r#type: FieldType::String, required: false, default: "" },
            runtime_connect_source: {
                r#type: FieldType::Enum(&["none", "operator"]),
                required: true,
                default: "none",
            },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ],

        connections: [
            node_id: {
                table: "pion_control_plane_node",
                on_delete: Cascade,
                model: "crate::generated::PionControlPlaneNode",
            },
        ]
    }
}
