use spectra_macros::spectra_schema;

spectra_schema! {
    PionMigrationLog {
        store: "pion",
        table: "pion_migration_log",
        version: "0.1.0",
        description: "Progress of the legacy control-plane table migration.",
        fields: [
            operation: {
                r#type: String,
                classification: { pii: false, safe_for_console: true },
            },
            from_table: {
                r#type: String,
                classification: { pii: false, safe_for_console: true },
            },
            to_table: {
                r#type: String,
                classification: { pii: false, safe_for_console: true },
            },
            rows_copied: {
                r#type: i64,
                classification: { pii: false, safe_for_console: true },
            },
            outcome: {
                r#type: String,
                classification: { pii: false, safe_for_console: true },
            },
            message: {
                r#type: String,
                classification: { pii: false, safe_for_console: true },
            },
        ],
    }
}
