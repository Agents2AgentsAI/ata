pub mod huggingface;
pub mod kaggle;

pub use huggingface::HuggingFaceClient;
pub use kaggle::KaggleClient;

/// Common result type for paginated API responses
#[derive(Debug, Clone)]
pub struct SearchPage<T> {
    pub items: Vec<T>,
    #[allow(dead_code)]
    pub total_available: Option<u64>,
    #[allow(dead_code)]
    pub has_more: bool,
}
