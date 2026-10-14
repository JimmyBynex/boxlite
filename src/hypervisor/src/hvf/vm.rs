// Copyright 2026 BoxLite Contributors
// SPDX-License-Identifier: Apache-2.0

use std::{io, ptr, sync::Mutex};

use super::ffi;
use super::memory::MemoryMap;
use crate::{Error, MemoryRegion, Result};

/// An arm64 HVF VM with an in-kernel GICv3.
///
/// HVF allows one VM per process. Memory backing remains owned by the caller,
/// under the same lifetime contract as [`crate::Vm::map_memory`]. Dropping this
/// value destroys the VM together with its GIC and memory mappings.
#[derive(Debug)]
pub struct HvfVm {
    memory: Mutex<MemoryMap>,
}

// `crate::Vm` requires Send + Sync.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<HvfVm>();
};

impl HvfVm {
    /// Creates the process's VM and its GIC.
    pub fn new() -> Result<Self> {
        Self::create().map_err(Error::CreateVm)
    }

    fn create() -> io::Result<Self> {
        // Needs no VM, so a host whose GIC does not fit fails before one exists.
        let layout = query_gic_layout()?;
        let mut ipa_bits = 0;
        // SAFETY: HVF writes one u32 through a pointer to a live local.
        let code = unsafe { ffi::hv_vm_config_get_default_ipa_size(&mut ipa_bits) };
        check("hv_vm_config_get_default_ipa_size", code)?;
        let memory = Mutex::new(MemoryMap::new(ipa_bits)?);

        // SAFETY: NULL selects the default configuration.
        let code = unsafe { ffi::hv_vm_create(ptr::null_mut()) };
        check("hv_vm_create", code)?;
        // Own the VM before anything else can fail: an error below drops `vm`,
        // and its Drop destroys the VM.
        let vm = Self { memory };

        create_gic(&layout)?;
        Ok(vm)
    }

    /// Maps caller-owned host memory into the guest, readable, writable and
    /// executable.
    ///
    /// # Safety
    ///
    /// The caller must uphold [`crate::Vm::map_memory`]'s requirements.
    pub unsafe fn map_memory(&self, region: &MemoryRegion) -> Result<()> {
        let install = || {
            // Keep validation, the HVF call and bookkeeping atomic against
            // other map/unmap calls; this does not protect guest RAM.
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| io::Error::other("HVF memory map lock is poisoned"))?;
            memory.map(region, |mapping| {
                let flags = ffi::HV_MEMORY_READ | ffi::HV_MEMORY_WRITE | ffi::HV_MEMORY_EXEC;
                // SAFETY: the caller guarantees the backing memory; MemoryMap
                // checked alignment, overflow, the IPA size and overlaps.
                let code = unsafe {
                    ffi::hv_vm_map(
                        region.host_addr.as_ptr().cast(),
                        mapping.guest_addr,
                        region.size,
                        flags,
                    )
                };
                check("hv_vm_map", code)
            })
        };
        install().map_err(|source| Error::MapMemory {
            guest_addr: region.guest_addr,
            size: region.size,
            source,
        })
    }

    /// Removes the mapping matching all three region fields, or fails with
    /// `NotFound`. A failed removal keeps the backing-memory requirements.
    pub fn unmap_memory(&self, region: &MemoryRegion) -> Result<()> {
        let remove = || {
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| io::Error::other("HVF memory map lock is poisoned"))?;
            memory.unmap(region, |mapping| {
                // SAFETY: integer arguments naming a recorded mapping; HVF
                // does not access host memory to remove it.
                let code = unsafe { ffi::hv_vm_unmap(mapping.guest_addr, region.size) };
                check("hv_vm_unmap", code)
            })
        };
        remove().map_err(|source| Error::UnmapMemory {
            guest_addr: region.guest_addr,
            size: region.size,
            source,
        })
    }

    /// Sets SPI `line` on the in-kernel GIC, from any thread.
    ///
    /// HVF rejects lines outside its SPI range with `InvalidInput`.
    pub fn set_irq_line(&self, line: u32, level: bool) -> Result<()> {
        // SAFETY: plain integer arguments; HVF accepts this call from any
        // thread once the GIC exists, which `new` guarantees.
        let code = unsafe { ffi::hv_gic_set_spi(line, level) };
        check("hv_gic_set_spi", code).map_err(|source| Error::SetIrqLine { line, source })
    }
}

impl Drop for HvfVm {
    fn drop(&mut self) {
        // SAFETY: only `create` builds this value, right after hv_vm_create
        // succeeds, so the process's VM exists and belongs to us.
        // Drop cannot report errors; on failure the VM keeps its mappings, so
        // backing memory must outlive the vCPUs.
        let _ = unsafe { ffi::hv_vm_destroy() };
    }
}

/// Queries HVF's GIC sizes and alignments, then places the GIC.
fn query_gic_layout() -> io::Result<GicLayout> {
    let (mut dist_size, mut dist_align, mut redist_size, mut redist_align) = (0, 0, 0, 0);
    // SAFETY (each query below): HVF writes one size_t through a pointer to a
    // live local.
    let code = unsafe { ffi::hv_gic_get_distributor_size(&mut dist_size) };
    check("hv_gic_get_distributor_size", code)?;
    let code = unsafe { ffi::hv_gic_get_distributor_base_alignment(&mut dist_align) };
    check("hv_gic_get_distributor_base_alignment", code)?;
    let code = unsafe { ffi::hv_gic_get_redistributor_region_size(&mut redist_size) };
    check("hv_gic_get_redistributor_region_size", code)?;
    let code = unsafe { ffi::hv_gic_get_redistributor_base_alignment(&mut redist_align) };
    check("hv_gic_get_redistributor_base_alignment", code)?;
    gic_layout(dist_size, dist_align, redist_size, redist_align)
}

/// Creates the GIC at `layout`.
fn create_gic(layout: &GicLayout) -> io::Result<()> {
    // Released when this function returns, on success or error; the GIC
    // keeps working after release.
    let config = GicConfig::new()?;
    // SAFETY: `config` holds a live config object until this function returns.
    let code = unsafe { ffi::hv_gic_config_set_distributor_base(config.0, layout.distributor) };
    check("hv_gic_config_set_distributor_base", code)?;
    // SAFETY: as above.
    let code = unsafe { ffi::hv_gic_config_set_redistributor_base(config.0, layout.redistributor) };
    check("hv_gic_config_set_redistributor_base", code)?;
    // SAFETY: as above; the caller has created the VM and no vCPU.
    let code = unsafe { ffi::hv_gic_create(config.0) };
    check("hv_gic_create", code)
}

struct GicConfig(ffi::hv_gic_config_t);

impl GicConfig {
    fn new() -> io::Result<Self> {
        // SAFETY: called after hv_vm_create.
        let config = unsafe { ffi::hv_gic_config_create() };
        if config.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "hv_gic_config_create returned NULL",
            ));
        }
        Ok(Self(config))
    }
}

impl Drop for GicConfig {
    fn drop(&mut self) {
        // SAFETY: `new` stored a non-NULL object returned retained by
        // hv_gic_config_create, and this is the only place that releases it.
        unsafe { ffi::os_release(self.0) };
    }
}

/// The VMM's arm64 memory map fixes the distributor just below the device window.
const GIC_DISTRIBUTOR_BASE: u64 = 0x3fff_0000;
const DEVICE_WINDOW_BASE: u64 = 0x4000_0000;

/// Guest physical bases of the GIC's two register windows.
#[derive(Debug, PartialEq, Eq)]
struct GicLayout {
    distributor: u64,
    redistributor: u64,
}

/// Places the redistributor region directly below the distributor.
///
/// HVF sizes that region for every vCPU it supports, not per guest vCPU, so the
/// base follows from the queried size; growing downwards keeps devices clear.
fn gic_layout(
    distributor_size: usize,
    distributor_alignment: usize,
    redistributor_region_size: usize,
    redistributor_alignment: usize,
) -> io::Result<GicLayout> {
    let unsupported = |message| Err(io::Error::new(io::ErrorKind::Unsupported, message));
    // A zero alignment fails is_multiple_of instead of dividing by zero.
    if !GIC_DISTRIBUTOR_BASE.is_multiple_of(distributor_alignment as u64)
        || GIC_DISTRIBUTOR_BASE
            .checked_add(distributor_size as u64)
            .is_none_or(|end| end > DEVICE_WINDOW_BASE)
    {
        return unsupported("HVF GIC distributor does not fit at 0x3fff0000");
    }
    match GIC_DISTRIBUTOR_BASE.checked_sub(redistributor_region_size as u64) {
        Some(base) if base.is_multiple_of(redistributor_alignment as u64) => Ok(GicLayout {
            distributor: GIC_DISTRIBUTOR_BASE,
            redistributor: base,
        }),
        _ => unsupported("HVF GIC redistributors do not fit below the distributor"),
    }
}

fn check(call: &'static str, code: ffi::hv_return_t) -> io::Result<()> {
    let kind = match code {
        ffi::HV_SUCCESS => return Ok(()),
        ffi::HV_DENIED => io::ErrorKind::PermissionDenied,
        ffi::HV_UNSUPPORTED => io::ErrorKind::Unsupported,
        ffi::HV_BUSY => io::ErrorKind::ResourceBusy,
        ffi::HV_BAD_ARGUMENT => io::ErrorKind::InvalidInput,
        ffi::HV_NO_RESOURCES => io::ErrorKind::OutOfMemory,
        _ => io::ErrorKind::Other,
    };
    // hv_return_t is not an errno, so from_raw_os_error would mislabel it.
    Err(io::Error::new(
        kind,
        format!("{call} returned {code:#010x}"),
    ))
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::*;

    /// Guest RAM from hv_vm_allocate, released on drop.
    struct Ram(MemoryRegion);

    impl Ram {
        fn new(guest_addr: u64, size: usize) -> Self {
            let mut addr = ptr::null_mut();
            // SAFETY: HVF writes the allocation's address through a pointer to a live local.
            check("hv_vm_allocate", unsafe {
                ffi::hv_vm_allocate(&mut addr, size, 0)
            })
            .unwrap();
            let host_addr = NonNull::new(addr.cast()).unwrap();
            Self(MemoryRegion {
                guest_addr,
                host_addr,
                size,
            })
        }
    }

    impl Drop for Ram {
        fn drop(&mut self) {
            // SAFETY: tests drop their VM, and with it every mapping, first.
            unsafe { ffi::hv_vm_deallocate(self.0.host_addr.as_ptr().cast(), self.0.size) };
        }
    }

    #[test]
    #[ignore = "requires Hypervisor.framework and the hypervisor entitlement"]
    fn mapped_ram_unmaps_only_with_its_exact_range() {
        // Declared before `vm`, so the VM and its mappings go first.
        let ram = Ram::new(0x8000_0000, 32 * KIB);
        let vm = HvfVm::new().unwrap();
        // SAFETY: `ram` outlives `vm`, and nothing else touches it.
        unsafe { vm.map_memory(&ram.0) }.unwrap();
        // HVF would report success for this partial range; BoxLite must not.
        let half = MemoryRegion {
            size: 16 * KIB,
            ..ram.0
        };
        assert!(matches!(
            vm.unmap_memory(&half),
            Err(Error::UnmapMemory { source, .. }) if source.kind() == io::ErrorKind::NotFound
        ));
        vm.unmap_memory(&ram.0).unwrap();
        // HVF rejects overlapping maps, so remapping proves the unmap reached it.
        // SAFETY: as above.
        unsafe { vm.map_memory(&ram.0) }.unwrap();
    }

    const KIB: usize = 1024;

    #[test]
    #[ignore = "requires Hypervisor.framework and the hypervisor entitlement"]
    fn dropping_the_vm_lets_the_process_create_another() {
        for _ in 0..2 {
            let vm = HvfVm::new().unwrap();
            vm.set_irq_line(32, true).unwrap();
            vm.set_irq_line(32, false).unwrap();
        }
    }

    #[test]
    #[ignore = "requires Hypervisor.framework and the hypervisor entitlement"]
    fn a_second_vm_is_busy_and_leaves_the_first_alive() {
        let vm = HvfVm::new().unwrap();
        assert!(matches!(
            HvfVm::new(),
            Err(Error::CreateVm(source)) if source.kind() == io::ErrorKind::ResourceBusy
        ));
        vm.set_irq_line(32, true).unwrap();
        vm.set_irq_line(32, false).unwrap();
    }

    #[test]
    #[ignore = "requires Hypervisor.framework and the hypervisor entitlement"]
    fn other_threads_toggle_spis_and_hvf_rejects_non_spi_lines() {
        let vm = HvfVm::new().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                vm.set_irq_line(32, true).unwrap();
                vm.set_irq_line(32, false).unwrap();
                // 31 is a private interrupt; GICv3 reserves INTIDs from 1020.
                for line in [31, 1020] {
                    assert!(matches!(
                        vm.set_irq_line(line, true),
                        Err(Error::SetIrqLine { line: failed, source })
                            if failed == line && source.kind() == io::ErrorKind::InvalidInput
                    ));
                }
            });
        });
    }

    #[test]
    fn redistributors_end_at_the_distributor_whatever_size_hvf_reports() {
        // Values measured on an M5 with macOS 26.4: a 32 MiB region, not 64 vCPUs x 128 KiB.
        assert_eq!(
            gic_layout(64 * KIB, 64 * KIB, 32 * KIB * KIB, 64 * KIB).unwrap(),
            GicLayout {
                distributor: 0x3fff_0000,
                redistributor: 0x3dff_0000,
            }
        );
    }

    #[test]
    fn layouts_that_overlap_devices_or_break_alignment_are_unsupported() {
        for (distributor_size, distributor_alignment, region_size, region_alignment) in [
            (128 * KIB, 64 * KIB, 32 * KIB * KIB, 64 * KIB), // distributor reaches the RTC
            (64 * KIB, 128 * KIB, 32 * KIB * KIB, 64 * KIB), // distributor misaligned
            (64 * KIB, 0, 32 * KIB * KIB, 64 * KIB),         // zero alignment
            (64 * KIB, 64 * KIB, 4 * KIB, 64 * KIB),         // redistributors misaligned
            (64 * KIB, 64 * KIB, 2048 * KIB * KIB, 64 * KIB), // region below address 0
        ] {
            assert_eq!(
                gic_layout(
                    distributor_size,
                    distributor_alignment,
                    region_size,
                    region_alignment
                )
                .unwrap_err()
                .kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn hvf_codes_become_io_error_kinds_naming_the_call_and_code() {
        check("hv_vm_create", ffi::HV_SUCCESS).unwrap();
        for (code, kind) in [
            (ffi::HV_DENIED, io::ErrorKind::PermissionDenied),
            (ffi::HV_UNSUPPORTED, io::ErrorKind::Unsupported),
            (ffi::HV_BUSY, io::ErrorKind::ResourceBusy),
            (ffi::HV_BAD_ARGUMENT, io::ErrorKind::InvalidInput),
            (ffi::HV_NO_RESOURCES, io::ErrorKind::OutOfMemory),
            // HV_ERROR: generic failure, e.g. an overlapping hv_vm_map.
            (0xfae9_4001_u32 as i32, io::ErrorKind::Other),
        ] {
            assert_eq!(check("hv_vm_create", code).unwrap_err().kind(), kind);
        }
        assert_eq!(
            check("hv_gic_create", ffi::HV_DENIED)
                .unwrap_err()
                .to_string(),
            "hv_gic_create returned 0xfae94007"
        );
    }
}
