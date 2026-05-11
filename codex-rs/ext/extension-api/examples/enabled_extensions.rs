#[path = "enabled_extensions/shared_state_extension.rs"]
mod shared_state_extension;

use std::sync::Arc;

use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistryBuilder;
use shared_state_extension::SharedStateExtension;
use shared_state_extension::recorded_style_contributions;
use shared_state_extension::recorded_usage_contributions;

fn main() {
    // 1. Build the extension value owned by the host.
    let extension = Arc::new(SharedStateExtension);

    // 2. Install it into the registry for the thread-start input type this host exposes.
    let registry = ExtensionRegistryBuilder::<()>::new()
        .with_extension(extension)
        .build();

    // 3. The host decides which stores are shared.
    let session_store = ExtensionData::new();
    let first_thread_store = ExtensionData::new();
    let second_thread_store = ExtensionData::new();

    // 4. Reusing the same session store shares session state across threads.
    let first_thread_fragments = contribute_prompt(&registry, &session_store, &first_thread_store);
    contribute_prompt(&registry, &session_store, &first_thread_store);
    contribute_prompt(&registry, &session_store, &second_thread_store);

    println!("first prompt fragments: {}", first_thread_fragments.len());
    println!(
        "session style contributions: {}",
        recorded_style_contributions(&session_store)
    );
    println!(
        "session usage contributions: {}",
        recorded_usage_contributions(&session_store)
    );
    println!(
        "first thread style contributions: {}",
        recorded_style_contributions(&first_thread_store)
    );
    println!(
        "first thread usage contributions: {}",
        recorded_usage_contributions(&first_thread_store)
    );
    println!(
        "second thread style contributions: {}",
        recorded_style_contributions(&second_thread_store)
    );
    println!(
        "second thread usage contributions: {}",
        recorded_usage_contributions(&second_thread_store)
    );
}

fn contribute_prompt(
    registry: &codex_extension_api::ExtensionRegistry<()>,
    session_store: &ExtensionData,
    thread_store: &ExtensionData,
) -> Vec<codex_extension_api::PromptFragment> {
    registry
        .context_contributors()
        .iter()
        .flat_map(|contributor| contributor.contribute(session_store, thread_store))
        .collect()
}
