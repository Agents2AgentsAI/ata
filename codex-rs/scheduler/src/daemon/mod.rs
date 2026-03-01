pub mod lifecycle;

pub use lifecycle::PidGuard;
pub use lifecycle::install_launchd;
pub use lifecycle::is_daemon_running;
pub use lifecycle::is_launchd_installed;
pub use lifecycle::start_daemon;
pub use lifecycle::start_daemon_background;
pub use lifecycle::stop_daemon;
pub use lifecycle::uninstall_launchd;
