use super::*;

pub(super) fn create_spawn_agents_on_csv_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "csv_path".to_string(),
        JsonSchema::String {
            description: Some("Path to the CSV file containing input rows.".to_string()),
        },
    );
    properties.insert(
        "instruction".to_string(),
        JsonSchema::String {
            description: Some(
                "Instruction template to apply to each CSV row. Use {column_name} placeholders to inject values from the row."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "id_column".to_string(),
        JsonSchema::String {
            description: Some("Optional column name to use as stable item id.".to_string()),
        },
    );
    properties.insert(
        "output_csv_path".to_string(),
        JsonSchema::String {
            description: Some("Optional output CSV path for exported results.".to_string()),
        },
    );
    properties.insert(
        "max_concurrency".to_string(),
        JsonSchema::Number {
            description: Some(
                "Maximum concurrent workers for this job. Defaults to 16 and is capped by config."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "max_workers".to_string(),
        JsonSchema::Number {
            description: Some(
                "Alias for max_concurrency. Set to 1 to run sequentially.".to_string(),
            ),
        },
    );
    properties.insert(
        "max_runtime_seconds".to_string(),
        JsonSchema::Number {
            description: Some(
                "Maximum runtime per worker before it is failed. Defaults to 1800 seconds."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "output_schema".to_string(),
        JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: None,
        },
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "spawn_agents_on_csv".to_string(),
        description: "Process a CSV by spawning one worker sub-agent per row. The instruction string is a template where `{column}` placeholders are replaced with row values. Each worker must call `report_agent_job_result` with a JSON object (matching `output_schema` when provided); missing reports are treated as failures. This call blocks until all rows finish and automatically exports results to `output_csv_path` (or a default path)."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["csv_path".to_string(), "instruction".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
}

pub(super) fn create_report_agent_job_result_tool() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "job_id".to_string(),
        JsonSchema::String {
            description: Some("Identifier of the job.".to_string()),
        },
    );
    properties.insert(
        "item_id".to_string(),
        JsonSchema::String {
            description: Some("Identifier of the job item.".to_string()),
        },
    );
    properties.insert(
        "result".to_string(),
        JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: None,
        },
    );
    properties.insert(
        "stop".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Optional. When true, cancels the remaining job items after this result is recorded."
                    .to_string(),
            ),
        },
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "report_agent_job_result".to_string(),
        description:
            "Worker-only tool to report a result for an agent job item. Main agents should not call this."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "job_id".to_string(),
                "item_id".to_string(),
                "result".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
}
