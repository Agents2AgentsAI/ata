//! Supabase integration for ATA account authentication and data access.

pub mod auth;
pub mod client;
pub mod error;
pub mod session;
pub mod types;

pub use auth::SupabaseAuth;
pub use client::SupabaseClient;
pub use error::SupabaseError;
pub use session::delete_ata_session;
pub use session::is_session_expired;
pub use session::load_ata_session;
pub use session::save_ata_session;
pub use types::DeviceRegistration;
pub use types::Profile;
pub use types::SupabaseSession;
pub use types::SupabaseUser;
