use valence::prelude::*;
use valence::privacy_policies::common::SYSTEM_ONLY;

valence_schema! {
    PionNodeActionCommand {
        repository: "https://github.com/unified-field-dev/pion",
        table: "pion_node_action_command",
        version: "0.1.0",
        database: crate::storage::GLUON_DEFAULT_STORAGE,
        description: "Queued action command targeting a specific agent node (pull/claim/execute). `payload_json` is Parton DTO JSON; deploy `command` may contain `{\"$secret_ref\":{id,version},\"field\":…}` until Pion claim resolves it.",

        policies: {
            read:   { allow: [SYSTEM_ONLY] },
            create: { allow: [SYSTEM_ONLY] },
            update: { allow: [SYSTEM_ONLY] },
            delete: { allow: [SYSTEM_ONLY] },
        },

        fields: [
            id: { r#type: FieldType::String, primary_key: true, required: true },
            node_id: { r#type: FieldType::String, required: true },
            cell_id: { r#type: FieldType::String, required: true },
            action_kind: { r#type: FieldType::String, required: true },
            payload_json: { r#type: FieldType::Json, required: true },
            status: {
                r#type: FieldType::Enum(&["pending", "running", "succeeded", "failed", "cancelled"]),
                required: true,
            },
            attempt: { r#type: FieldType::Integer, required: true },
            max_attempts: { r#type: FieldType::Integer, required: true },
            lease_expires_at: { r#type: FieldType::DateTime },
            correlation_key: { r#type: FieldType::String, required: true },
            sequence: { r#type: FieldType::Integer, required: true },
            last_error: { r#type: FieldType::String },
            // F12: random per-claim-attempt fencing token, set by `claim_pending_node_action` on
            // every pending->running transition and re-checked immediately after commit to detect
            // a lost race against a concurrent claimer. `lease_expires_at` can't be used for this
            // — it round-trips through storage at whole-second precision, so two claims committed
            // within the same second are indistinguishable by timestamp alone.
            claim_fence: { r#type: FieldType::String },
            created_at: { r#type: FieldType::DateTime, required: true },
            updated_at: { r#type: FieldType::DateTime, required: true },
        ]
    }
}
