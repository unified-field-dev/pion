use valence::prelude::*;
use valence::privacy_policies::common::{AUTHENTICATED, SYSTEM_ONLY};

valence_schema! {
    PionControlPlaneAuthority {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_control_plane_authority",
        version: "0.1.1",
        database: crate::storage::CONTROL_PLANE_DEFAULT_STORAGE,
        description: "Stable control-plane authority URL and Gluon-managed proxy routing (HAProxy v1)",

        policies: {
            read:   { allow: [AUTHENTICATED] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            public_base_url: { r#type: FieldType::String, required: true },
            proxy_kind: {
                r#type: FieldType::Enum(&["haproxy"]),
                required: true,
            },
            haproxy_container_name: { r#type: FieldType::String, required: true, default: "gluon-cp-authority" },
            haproxy_config_relative_path: { r#type: FieldType::String, required: true, default: "haproxy/cp_authority.cfg" },
            listen_port: { r#type: FieldType::Integer, required: true, default: 8443 },
            routing_phase: {
                r#type: FieldType::Enum(&["old_only", "overlap", "new_only"]),
                required: true,
                default: "old_only",
            },
            old_backends_json: { r#type: FieldType::Json, required: true },
            new_backends_json: { r#type: FieldType::Json, required: true },
            config_generation: { r#type: FieldType::Integer, required: true, default: 0 },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
            signing_pubkey: { r#type: FieldType::String, required: false, default: "" },
            directive_signing_secret_id: { r#type: FieldType::String, required: false, default: "" },
            directive_signing_secret_version: { r#type: FieldType::Integer, required: false, default: 0 },
        ]
    }
}
