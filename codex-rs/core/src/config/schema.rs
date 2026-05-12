// Re-export base schema utilities from codex_config.
pub use codex_config::schema::canonicalize;

use schemars::schema::InstanceType;
use schemars::schema::ObjectValidation;
use schemars::schema::Schema;
use schemars::schema::SchemaObject;
use std::path::Path;

/// Schema for the top-level `treesitter` key.
///
/// The value is an untagged enum in Rust (`TreeSitterConfig` in `code_intel.rs`)
/// that accepts either a plain boolean (enable/disable) or a configuration
/// object with detailed settings. Because the config is consumed from the TOML
/// layer stack rather than a typed `ConfigToml` field, inject the schema here as
/// a post-processing step.
fn treesitter_config_schema() -> SchemaObject {
    let bool_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Boolean.into()),
        ..Default::default()
    });

    let mut obj_props = schemars::Map::new();
    obj_props.insert(
        "max_file_size".to_string(),
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Integer.into()),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(0.0),
                ..Default::default()
            })),
            ..Default::default()
        }),
    );
    for arr_key in &["ignore_patterns", "ignore_extensions", "disabled_languages"] {
        obj_props.insert(
            arr_key.to_string(),
            Schema::Object(SchemaObject {
                instance_type: Some(InstanceType::Array.into()),
                array: Some(Box::new(schemars::schema::ArrayValidation {
                    items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                        Schema::Object(SchemaObject {
                            instance_type: Some(InstanceType::String.into()),
                            ..Default::default()
                        }),
                    ))),
                    ..Default::default()
                })),
                metadata: Some(Box::new(schemars::schema::Metadata {
                    default: Some(serde_json::json!([])),
                    ..Default::default()
                })),
                ..Default::default()
            }),
        );
    }
    obj_props.insert(
        "annotation_store_path".to_string(),
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        }),
    );
    obj_props.insert(
        "persist_annotations".to_string(),
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Boolean.into()),
            metadata: Some(Box::new(schemars::schema::Metadata {
                default: Some(serde_json::json!(true)),
                ..Default::default()
            })),
            ..Default::default()
        }),
    );

    let obj_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            properties: obj_props,
            additional_properties: Some(Box::new(Schema::Bool(false))),
            ..Default::default()
        })),
        ..Default::default()
    });

    let null_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Null.into()),
        ..Default::default()
    });

    SchemaObject {
        metadata: Some(Box::new(schemars::schema::Metadata {
            description: Some("TreeSitter indexing configuration.".to_string()),
            default: Some(serde_json::json!(null)),
            ..Default::default()
        })),
        subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
            any_of: Some(vec![bool_schema, obj_schema, null_schema]),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Schema for the top-level `lsp` key.
fn lsp_config_schema() -> SchemaObject {
    let bool_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Boolean.into()),
        ..Default::default()
    });

    let full_server_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            properties: schemars::Map::from_iter([
                (
                    "command".to_string(),
                    Schema::Object(SchemaObject {
                        instance_type: Some(InstanceType::Array.into()),
                        array: Some(Box::new(schemars::schema::ArrayValidation {
                            items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                                Schema::Object(SchemaObject {
                                    instance_type: Some(InstanceType::String.into()),
                                    ..Default::default()
                                }),
                            ))),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                ),
                (
                    "extensions".to_string(),
                    Schema::Object(SchemaObject {
                        instance_type: Some(InstanceType::Array.into()),
                        array: Some(Box::new(schemars::schema::ArrayValidation {
                            items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                                Schema::Object(SchemaObject {
                                    instance_type: Some(InstanceType::String.into()),
                                    ..Default::default()
                                }),
                            ))),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                ),
                (
                    "root_markers".to_string(),
                    Schema::Object(SchemaObject {
                        instance_type: Some(InstanceType::Array.into()),
                        array: Some(Box::new(schemars::schema::ArrayValidation {
                            items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                                Schema::Object(SchemaObject {
                                    instance_type: Some(InstanceType::String.into()),
                                    ..Default::default()
                                }),
                            ))),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                ),
                (
                    "env".to_string(),
                    Schema::Object(SchemaObject {
                        instance_type: Some(InstanceType::Object.into()),
                        object: Some(Box::new(ObjectValidation {
                            additional_properties: Some(Box::new(Schema::Object(SchemaObject {
                                instance_type: Some(InstanceType::String.into()),
                                ..Default::default()
                            }))),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                ),
                ("initialization_options".to_string(), Schema::Bool(true)),
                (
                    "disabled".to_string(),
                    Schema::Object(SchemaObject {
                        instance_type: Some(InstanceType::Boolean.into()),
                        ..Default::default()
                    }),
                ),
            ]),
            additional_properties: Some(Box::new(Schema::Bool(false))),
            ..Default::default()
        })),
        ..Default::default()
    });

    let disabled_only_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            properties: schemars::Map::from_iter([(
                "disabled".to_string(),
                Schema::Object(SchemaObject {
                    instance_type: Some(InstanceType::Boolean.into()),
                    ..Default::default()
                }),
            )]),
            required: vec!["disabled".to_string()].into_iter().collect(),
            additional_properties: Some(Box::new(Schema::Bool(false))),
            ..Default::default()
        })),
        ..Default::default()
    });

    let object_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            additional_properties: Some(Box::new(Schema::Object(SchemaObject {
                subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                    any_of: Some(vec![full_server_schema, disabled_only_schema]),
                    ..Default::default()
                })),
                ..Default::default()
            }))),
            ..Default::default()
        })),
        ..Default::default()
    });

    let null_schema = Schema::Object(SchemaObject {
        instance_type: Some(InstanceType::Null.into()),
        ..Default::default()
    });

    SchemaObject {
        metadata: Some(Box::new(schemars::schema::Metadata {
            description: Some("LSP server configuration.".to_string()),
            default: Some(serde_json::json!(null)),
            ..Default::default()
        })),
        subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
            any_of: Some(vec![bool_schema, object_schema, null_schema]),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Build the config schema for `config.toml`, extending the upstream schema
/// with ATA-specific `lsp` and `treesitter` entries.
pub fn config_schema() -> schemars::schema::RootSchema {
    let mut schema = codex_config::schema::config_schema();

    schema
        .schema
        .object
        .get_or_insert_with(Default::default)
        .properties
        .insert("lsp".to_string(), Schema::Object(lsp_config_schema()));
    schema
        .schema
        .object
        .get_or_insert_with(Default::default)
        .properties
        .insert(
            "treesitter".to_string(),
            Schema::Object(treesitter_config_schema()),
        );

    schema
}

/// Render the config schema as pretty-printed JSON (with ATA extensions).
pub fn config_schema_json() -> anyhow::Result<Vec<u8>> {
    let schema = config_schema();
    let value = serde_json::to_value(schema)?;
    let value = canonicalize(&value);
    let json = serde_json::to_vec_pretty(&value)?;
    Ok(json)
}

/// Write the ATA-extended config schema fixture to disk.
pub fn write_config_schema(out_path: &Path) -> anyhow::Result<()> {
    let json = config_schema_json()?;
    std::fs::write(out_path, json)?;
    Ok(())
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
