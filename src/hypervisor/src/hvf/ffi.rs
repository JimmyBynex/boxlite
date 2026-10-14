// Copyright 2026 BoxLite Contributors
// SPDX-License-Identifier: Apache-2.0

//! Hand-written Hypervisor.framework declarations for the calls this backend uses.
//!
//! Header references are relative to
//! `$SDK/System/Library/Frameworks/Hypervisor.framework/Headers/`, except
//! `hv_kern_types.h` in `$SDK/usr/include/arm64/hv/`. The x86-only `hv_error.h`
//! assigns different names to the same codes.

#![allow(non_camel_case_types)]

use std::ffi::c_void;

/// `hv_kern_types.h:63`: `mach_error_t`, a C `int`.
pub(super) type hv_return_t = i32;
/// `hv_vm_types.h:32`: a guest physical address.
pub(super) type hv_ipa_t = u64;
/// `hv_vm_types.h:26`: opaque; only ever passed as NULL for the default config.
pub(super) type hv_vm_config_t = *mut c_void;
/// `hv_gic_types.h:25`: opaque OS object, released with `os_release`.
pub(super) type hv_gic_config_t = *mut c_void;

// `hv_kern_types.h:50-60`. The codes exceed `i32::MAX`, so they wrap as in C.
pub(super) const HV_SUCCESS: hv_return_t = 0;
pub(super) const HV_BUSY: hv_return_t = 0xfae9_4002_u32 as i32;
pub(super) const HV_BAD_ARGUMENT: hv_return_t = 0xfae9_4003_u32 as i32;
pub(super) const HV_NO_RESOURCES: hv_return_t = 0xfae9_4005_u32 as i32;
pub(super) const HV_DENIED: hv_return_t = 0xfae9_4007_u32 as i32;
pub(super) const HV_UNSUPPORTED: hv_return_t = 0xfae9_400f_u32 as i32;

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    /// `hv_vm.h:33`. One VM per process; NULL selects the default configuration.
    pub(super) fn hv_vm_create(config: hv_vm_config_t) -> hv_return_t;
    /// `hv_vm.h:42`. Fails while any vCPU exists (`hv_vm.h:39`).
    pub(super) fn hv_vm_destroy() -> hv_return_t;

    // C `size_t` is Rust `usize`.
    /// `hv_gic_parameters.h:26`
    pub(super) fn hv_gic_get_distributor_size(size: *mut usize) -> hv_return_t;
    /// `hv_gic_parameters.h:34`
    pub(super) fn hv_gic_get_distributor_base_alignment(alignment: *mut usize) -> hv_return_t;
    /// `hv_gic_parameters.h:46`: covers every vCPU the VM supports.
    pub(super) fn hv_gic_get_redistributor_region_size(size: *mut usize) -> hv_return_t;
    /// `hv_gic_parameters.h:62`
    pub(super) fn hv_gic_get_redistributor_base_alignment(alignment: *mut usize) -> hv_return_t;

    /// `hv_gic_config.h:28`. Call after `hv_vm_create` (`:24`); may return NULL.
    pub(super) fn hv_gic_config_create() -> hv_gic_config_t;
    /// `hv_gic_config.h:40`
    pub(super) fn hv_gic_config_set_distributor_base(
        config: hv_gic_config_t,
        base: hv_ipa_t,
    ) -> hv_return_t;
    /// `hv_gic_config.h:54`
    pub(super) fn hv_gic_config_set_redistributor_base(
        config: hv_gic_config_t,
        base: hv_ipa_t,
    ) -> hv_return_t;
    /// `hv_gic.h:49`. After `hv_vm_create`, before any vCPU (`hv_gic.h:35-38`).
    pub(super) fn hv_gic_create(config: hv_gic_config_t) -> hv_return_t;
    /// `hv_gic.h:66`. C `bool` and Rust `bool` share one ABI.
    pub(super) fn hv_gic_set_spi(intid: u32, level: bool) -> hv_return_t;
}

// libSystem, which std already links.
unsafe extern "C" {
    /// `os/object.h:321`
    pub(super) fn os_release(object: *mut c_void);
}
