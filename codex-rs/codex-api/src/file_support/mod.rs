pub mod capabilities;
pub mod data_url;
pub mod responses;
pub mod routing;
pub mod upload;

pub(crate) const INPUT_FILE_BLOCK_MISSING_DATA_OR_ID: &str =
    "input_file block must include file_data or file_id";

pub use capabilities::FileCapabilityConfig;
pub use capabilities::file_capabilities_for;
pub use data_url::build_data_url;
pub use data_url::parse_data_url;
pub use responses::wrap_responses_input_file_data_uris;
pub use routing::FileRoutingError;
pub use routing::route_file_input;
pub use upload::FileUploadError;
pub use upload::FileUploadService;
pub use upload::UploadedFile;
