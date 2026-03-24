use super::*;

// @agent-facing
pub(super) fn create_crop_and_store_figure_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "pdf_url".to_string(),
            JsonSchema::String {
                description: Some(
                    "The URL of the PDF (must already be attached via attach_url_files)."
                        .to_string(),
                ),
            },
        ),
        (
            "page".to_string(),
            JsonSchema::Number {
                description: Some("1-indexed page number containing the figure.".to_string()),
            },
        ),
        (
            "x".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Left edge of the bounding box as a fraction of page width (0.0\u{2013}1.0)."
                        .to_string(),
                ),
            },
        ),
        (
            "y".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Top edge of the bounding box as a fraction of page height (0.0\u{2013}1.0)."
                        .to_string(),
                ),
            },
        ),
        (
            "w".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Width of the bounding box as a fraction of page width (0.0\u{2013}1.0)."
                        .to_string(),
                ),
            },
        ),
        (
            "h".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Height of the bounding box as a fraction of page height (0.0\u{2013}1.0)."
                        .to_string(),
                ),
            },
        ),
        (
            "caption".to_string(),
            JsonSchema::String {
                description: Some(
                    "Caption for the figure (e.g. 'Figure 3: Architecture overview').".to_string(),
                ),
            },
        ),
        (
            "description".to_string(),
            JsonSchema::String {
                description: Some(
                    "Brief description of what the figure shows for accessibility and indexing."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "crop_and_store_figure".to_string(),
        description: "Extract a figure from a PDF by specifying its page and bounding box. \
             IMPORTANT: Coordinates are fractions (0.0\u{2013}1.0) of the FULL page \
             including margins, where (0,0) is the absolute top-left corner of the \
             page and (1,1) is the absolute bottom-right. Academic papers typically \
             have ~15\u{2013}20% margins on each side, so a figure centered in the \
             content area would start around x=0.15\u{2013}0.20, not x=0.0. \
             A small padding is automatically added around your coordinates, so \
             prefer tight bounding boxes. \
             The PDF must already be attached via attach_url_files. \
             Returns the asset path to reference in reading view markdown \
             as ![caption](asset_path)."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "pdf_url".to_string(),
                "page".to_string(),
                "x".to_string(),
                "y".to_string(),
                "w".to_string(),
                "h".to_string(),
                "caption".to_string(),
                "description".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

// @agent-facing
pub(super) fn create_view_image_tool(can_request_original_image_detail: bool) -> ToolSpec {
    let mut properties = BTreeMap::from([(
        "path".to_string(),
        JsonSchema::String {
            description: Some("Local filesystem path to an image file".to_string()),
        },
    )]);
    if can_request_original_image_detail {
        properties.insert(
            "detail".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional detail override. The only supported value is `original`; omit this field for default resized behavior. Use `original` to preserve the file's original resolution instead of resizing to fit. This is important when high-fidelity image perception or precise localization is needed, especially for CUA agents.".to_string(),
                ),
            },
        );
    }

    ToolSpec::Function(ResponsesApiTool {
        name: VIEW_IMAGE_TOOL_NAME.to_string(),
        description: "View a local image from the filesystem (only use if given a full filepath by the user, and the image isn't already attached to the thread context within <image ...> tags)."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["path".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub(super) fn create_test_sync_tool() -> ToolSpec {
    let barrier_properties = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::String {
                description: Some(
                    "Identifier shared by concurrent calls that should rendezvous".to_string(),
                ),
            },
        ),
        (
            "participants".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Number of tool calls that must arrive before the barrier opens".to_string(),
                ),
            },
        ),
        (
            "timeout_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum time in milliseconds to wait at the barrier".to_string(),
                ),
            },
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "sleep_before_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Optional delay in milliseconds before any other action".to_string(),
                ),
            },
        ),
        (
            "sleep_after_ms".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Optional delay in milliseconds after completing the barrier".to_string(),
                ),
            },
        ),
        (
            "barrier".to_string(),
            JsonSchema::Object {
                properties: barrier_properties,
                required: Some(vec!["id".to_string(), "participants".to_string()]),
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "test_sync_tool".to_string(),
        description: "Internal synchronization helper used by Codex integration tests.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

// @agent-facing
pub(super) fn create_grep_files_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "pattern".to_string(),
            JsonSchema::String {
                description: Some("Regular expression pattern to search for.".to_string()),
            },
        ),
        (
            "include".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional glob that limits which files are searched (e.g. \"*.rs\" or \
                     \"*.{ts,tsx}\")."
                        .to_string(),
                ),
            },
        ),
        (
            "path".to_string(),
            JsonSchema::String {
                description: Some(
                    "Directory or file path to search. Defaults to the session's working directory."
                        .to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Maximum number of file paths to return (defaults to 100).".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "grep_files".to_string(),
        description: "Finds files whose contents match the pattern and lists them by modification \
                      time."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["pattern".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

// @agent-facing
pub(super) fn create_read_file_tool() -> ToolSpec {
    let indentation_properties = BTreeMap::from([
        (
            "anchor_line".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Anchor line to center the indentation lookup on (defaults to offset)."
                        .to_string(),
                ),
            },
        ),
        (
            "max_levels".to_string(),
            JsonSchema::Number {
                description: Some(
                    "How many parent indentation levels (smaller indents) to include.".to_string(),
                ),
            },
        ),
        (
            "include_siblings".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "When true, include additional blocks that share the anchor indentation."
                        .to_string(),
                ),
            },
        ),
        (
            "include_header".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Include doc comments or attributes directly above the selected block."
                        .to_string(),
                ),
            },
        ),
        (
            "max_lines".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Hard cap on the number of lines returned when using indentation mode."
                        .to_string(),
                ),
            },
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "file_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute path to the file".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The line number to start reading from. Must be 1 or greater.".to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("The maximum number of lines to return.".to_string()),
            },
        ),
        (
            "mode".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional mode selector: \"slice\" for simple ranges (default) or \"indentation\" \
                     to expand around an anchor line."
                        .to_string(),
                ),
            },
        ),
        (
            "indentation".to_string(),
            JsonSchema::Object {
                properties: indentation_properties,
                required: None,
                additional_properties: Some(false.into()),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "read_file".to_string(),
        description:
            "Reads a local file with 1-indexed line numbers, supporting slice and indentation-aware block modes."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file_path".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

// @agent-facing
pub(super) fn create_list_dir_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "dir_path".to_string(),
            JsonSchema::String {
                description: Some("Absolute path to the directory to list.".to_string()),
            },
        ),
        (
            "offset".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The entry number to start listing from. Must be 1 or greater.".to_string(),
                ),
            },
        ),
        (
            "limit".to_string(),
            JsonSchema::Number {
                description: Some("The maximum number of entries to return.".to_string()),
            },
        ),
        (
            "depth".to_string(),
            JsonSchema::Number {
                description: Some(
                    "The maximum directory depth to traverse. Must be 1 or greater.".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_dir".to_string(),
        description:
            "Lists entries in a local directory with 1-indexed entry numbers and simple type labels."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["dir_path".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}
