//! Bonjour/mDNS service advertisement for the embedded remote-control server.
//!
//! Advertises `_codex-remote._tcp` on the local network so that mobile clients
//! can discover the TUI without manual IP/port entry.

use std::collections::HashMap;
use std::net::IpAddr;

use mdns_sd::ServiceDaemon;
use mdns_sd::ServiceInfo;
use tracing::error;
use tracing::info;

/// mDNS service type for Codex remote control.
const SERVICE_TYPE: &str = "_codex-remote._tcp.local.";

/// Handle for the advertised mDNS service. Dropping this stops advertisement.
pub(crate) struct RemoteDiscoveryHandle {
    daemon: ServiceDaemon,
    fullname: String,
}

impl RemoteDiscoveryHandle {
    /// Stop advertising the service.
    pub fn stop(&self) {
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            error!("failed to unregister mDNS service: {e}");
        }
    }
}

impl Drop for RemoteDiscoveryHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start advertising the remote-control service via mDNS/Bonjour.
///
/// Returns a handle that keeps the advertisement alive. Drop it to stop.
pub(crate) fn advertise_remote_service(
    port: u16,
    auth_token: &str,
    lan_ip: IpAddr,
) -> Option<RemoteDiscoveryHandle> {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            error!("failed to create mDNS daemon: {e}");
            return None;
        }
    };

    let hostname = gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| "codex-tui".to_string());

    // TXT records: protocol version and token hint (first 4 chars).
    let mut properties = HashMap::new();
    properties.insert("version".to_string(), "1".to_string());
    properties.insert(
        "token_hint".to_string(),
        auth_token[..auth_token.len().min(4)].to_string(),
    );
    properties.insert("hostname".to_string(), hostname.clone());

    let instance_name = format!("Codex TUI on {hostname}");

    let service_info = match ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &format!("{hostname}.local."),
        lan_ip,
        port,
        properties,
    ) {
        Ok(info) => info,
        Err(e) => {
            error!("failed to create mDNS service info: {e}");
            return None;
        }
    };

    let fullname = service_info.get_fullname().to_string();

    if let Err(e) = daemon.register(service_info) {
        error!("failed to register mDNS service: {e}");
        return None;
    }

    info!("mDNS: advertising {SERVICE_TYPE} as '{instance_name}' on {lan_ip}:{port}");

    Some(RemoteDiscoveryHandle { daemon, fullname })
}
