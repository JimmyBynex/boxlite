// Copyright 2026 BoxLite Contributors
// SPDX-License-Identifier: Apache-2.0

//! Hand-written Hypervisor.framework declarations for the calls this backend uses.
//!
//! Headers live in `$SDK/System/Library/Frameworks/Hypervisor.framework/Headers/`,
//! except `hv_kern_types.h` in `$SDK/usr/include/arm64/hv/`.

#![allow(non_camel_case_types)]

use std::ffi::c_void;

pub(super) type hv_return_t = i32;
pub(super) type hv_ipa_t = u64;
/// Opaque; only ever passed as NULL for the default config.
pub(super) type hv_vm_config_t = *mut c_void;
/// Opaque OS object, released with `os_release`.
pub(super) type hv_gic_config_t = *mut c_void;
/// arm64 values from `hv_kern_types.h`.
pub(super) type hv_memory_flags_t = u64;
pub(super) const HV_MEMORY_READ: hv_memory_flags_t = 1 << 0;
pub(super) const HV_MEMORY_WRITE: hv_memory_flags_t = 1 << 1;
pub(super) const HV_MEMORY_EXEC: hv_memory_flags_t = 1 << 2;

// `hv_kern_types.h`. The codes exceed `i32::MAX`, so they wrap as in C.
pub(super) const HV_SUCCESS: hv_return_t = 0;
pub(super) const HV_BUSY: hv_return_t = 0xfae9_4002_u32 as i32;
pub(super) const HV_BAD_ARGUMENT: hv_return_t = 0xfae9_4003_u32 as i32;
pub(super) const HV_NO_RESOURCES: hv_return_t = 0xfae9_4005_u32 as i32;
pub(super) const HV_DENIED: hv_return_t = 0xfae9_4007_u32 as i32;
pub(super) const HV_UNSUPPORTED: hv_return_t = 0xfae9_400f_u32 as i32;

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    /// NULL selects the default configuration.
    pub(super) fn hv_vm_create(config: hv_vm_config_t) -> hv_return_t;
    /// Fails while any vCPU exists.
    pub(super) fn hv_vm_destroy() -> hv_return_t;
    /// The IPA size `hv_vm_create(NULL)` uses.
    pub(super) fn hv_vm_config_get_default_ipa_size(bits: *mut u32) -> hv_return_t;
    /// Page-aligned addresses and size.
    pub(super) fn hv_vm_map(
        addr: *mut c_void,
        ipa: hv_ipa_t,
        size: usize,
        flags: hv_memory_flags_t,
    ) -> hv_return_t;
    pub(super) fn hv_vm_unmap(ipa: hv_ipa_t, size: usize) -> hv_return_t;
    /// Page-aligned anonymous memory for guest RAM.
    #[cfg(test)]
    pub(super) fn hv_vm_allocate(addr: *mut *mut c_void, size: usize, flags: u64) -> hv_return_t;
    #[cfg(test)]
    pub(super) fn hv_vm_deallocate(addr: *mut c_void, size: usize) -> hv_return_t;

    pub(super) fn hv_gic_get_distributor_size(size: *mut usize) -> hv_return_t;
    pub(super) fn hv_gic_get_distributor_base_alignment(alignment: *mut usize) -> hv_return_t;
    pub(super) fn hv_gic_get_redistributor_region_size(size: *mut usize) -> hv_return_t;
    pub(super) fn hv_gic_get_redistributor_base_alignment(alignment: *mut usize) -> hv_return_t;

    /// Call after `hv_vm_create`; may return NULL.
    pub(super) fn hv_gic_config_create() -> hv_gic_config_t;
    pub(super) fn hv_gic_config_set_distributor_base(
        config: hv_gic_config_t,
        base: hv_ipa_t,
    ) -> hv_return_t;
    pub(super) fn hv_gic_config_set_redistributor_base(
        config: hv_gic_config_t,
        base: hv_ipa_t,
    ) -> hv_return_t;
    /// Call after `hv_vm_create`, before any vCPU.
    pub(super) fn hv_gic_create(config: hv_gic_config_t) -> hv_return_t;
    pub(super) fn hv_gic_set_spi(intid: u32, level: bool) -> hv_return_t;
}

// libSystem, which std already links.
unsafe extern "C" {
    pub(super) fn os_release(object: *mut c_void);
}
