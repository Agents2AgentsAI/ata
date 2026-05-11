use std::sync::Arc;

use crate::ExtensionRegistryBuilder;

/// First-party extension that can install one or more typed runtime contributions.
///
/// Implementations should use [`Self::install`] only to register the concrete
/// providers they own.
pub trait CodexExtension<C>: Send + Sync {
    /// Registers this extension's concrete typed contributions.
    fn install(self: Arc<Self>, registry: &mut ExtensionRegistryBuilder<C>);
}
