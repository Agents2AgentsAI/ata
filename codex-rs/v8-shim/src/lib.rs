//! Symbol shim for the prebuilt rusty_v8 archive on Linux.
//!
//! The rusty_v8 prebuilt openai/codex publishes for v8 v147+ (the
//! `ptrcomp_sandbox_release` variant) was built against a newer libc++ that
//! exports `std::__1::__hash_memory(const void*, size_t)`. Ubuntu noble's
//! default `libc++-18` and even `libc++-20` packages still keep that helper
//! header-only inline, so the symbol is missing from `libc++.so` and the
//! final binary link fails on Linux Tests jobs.
//!
//! This crate exposes the symbol via a `no_mangle` Rust function whose
//! unmangled name matches the Itanium-mangled C++ name. The actual hash
//! algorithm only needs to be deterministic for the lifetime of the process
//! — v8 uses it for internal `unordered_map` buckets, not for any
//! externally observable hash value — so a small FNV-1a is sufficient.
//!
//! Crates that pull in the prebuilt v8 (`codex-code-mode`, `codex-v8-poc`)
//! depend on `codex-v8-shim` so the symbol shows up in every binary that
//! transitively links libv8. Living in its own crate also keeps the symbol
//! defined exactly once even when both consumers are linked together.

#[cfg(target_os = "linux")]
mod hash_memory_shim {
    #![allow(non_snake_case)]

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn _ZNSt3__113__hash_memoryEPKvm(
        ptr: *const core::ffi::c_void,
        size: usize,
    ) -> usize {
        let bytes = unsafe { core::slice::from_raw_parts(ptr.cast::<u8>(), size) };
        let mut hash: u64 = 0xcbf29ce4_84222325;
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash as usize
    }
}
