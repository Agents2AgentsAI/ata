// Linux-only shim: provide a definition of `std::__1::__hash_memory(const
// void*, size_t)` so the prebuilt rusty_v8 archive (built against a newer
// libc++ that exports the symbol) links against Ubuntu noble's libc++-18
// / libc++-20 packages, where the same function is still header-only inline
// and so absent from `libc++.so`.
//
// We define it as a `#[no_mangle]` Rust function whose unmangled symbol name
// matches the Itanium-mangled C++ symbol that v8 expects. The actual hash
// algorithm only needs to be deterministic for the lifetime of the process —
// v8 uses it for internal unordered_map buckets — so a small FNV-1a is
// sufficient.

#![cfg(target_os = "linux")]
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
