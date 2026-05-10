// Linux-only shim: provide a definition of `std::__1::__hash_memory` so the
// prebuilt rusty_v8 archive (built against a newer libc++ that exports the
// symbol) links against Ubuntu noble's libc++-18/libc++-20 packages, where
// the same function is still header-only inline and so absent from
// `libc++.so`.
//
// The actual hash algorithm only needs to be deterministic for the lifetime
// of the process — v8 uses it for internal unordered_map buckets, not for
// any externally-visible hash value — so a small FNV-1a implementation is
// sufficient.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "linux" {
        return;
    }

    let shim = r#"
#include <cstddef>
#include <cstdint>

extern "C" {

#if defined(__GNUC__) || defined(__clang__)
[[gnu::weak]]
#endif
std::size_t _ZNSt3__113__hash_memoryEPKvm(const void* ptr, std::size_t size) noexcept {
    const unsigned char* data = static_cast<const unsigned char*>(ptr);
    std::uint64_t hash = 0xcbf29ce484222325ULL;
    for (std::size_t i = 0; i < size; ++i) {
        hash ^= static_cast<std::uint64_t>(data[i]);
        hash = hash * 0x100000001b3ULL;
    }
    return static_cast<std::size_t>(hash);
}

} // extern "C"
"#;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let shim_path = std::path::Path::new(&out_dir).join("hash_memory_shim.cpp");
    std::fs::write(&shim_path, shim).expect("write hash_memory_shim.cpp");

    cc::Build::new()
        .cpp(true)
        .file(&shim_path)
        .flag_if_supported("-std=c++17")
        .compile("codex_hash_memory_shim");

    println!("cargo:rerun-if-changed=build.rs");
}
