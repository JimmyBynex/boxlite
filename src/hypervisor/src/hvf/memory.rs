// Copyright 2026 BoxLite Contributors
// SPDX-License-Identifier: Apache-2.0

//! Guest memory bookkeeping, private to HVF.
//!
//! `hv_vm_unmap` reports success for any range, mapped or not, so these records
//! are the only check that an unmap names exactly what was mapped.

use std::io;

use crate::MemoryRegion;

/// The Apple silicon host page, which HVF's default 16 KiB granule also requires.
const PAGE_SIZE: u64 = 16 * 1024;

/// One guest mapping. Addresses are integers so `HvfVm` stays `Send + Sync`;
/// `MemoryRegion`'s `NonNull` is neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Mapping {
    pub(super) guest_addr: u64,
    pub(super) size: u64,
    pub(super) host_addr: usize,
}

impl From<&MemoryRegion> for Mapping {
    fn from(region: &MemoryRegion) -> Self {
        Self {
            guest_addr: region.guest_addr,
            size: region.size as u64,
            host_addr: region.host_addr.as_ptr() as usize,
        }
    }
}

#[derive(Debug)]
pub(super) struct MemoryMap {
    mappings: Vec<Mapping>,
    /// One past the highest guest address the VM's IPA size can reach.
    ipa_limit: u64,
}

impl MemoryMap {
    pub(super) fn new(ipa_bits: u32) -> io::Result<Self> {
        let ipa_limit = 1u64.checked_shl(ipa_bits).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("HVF reports a {ipa_bits}-bit IPA size"),
            )
        })?;
        Ok(Self {
            mappings: Vec::new(),
            ipa_limit,
        })
    }

    /// Validates `region`, calls `install` and records the mapping only if it succeeds.
    pub(super) fn map(
        &mut self,
        region: &MemoryRegion,
        install: impl FnOnce(Mapping) -> io::Result<()>,
    ) -> io::Result<()> {
        let mapping = Mapping::from(region);
        let end = self.validate(mapping)?;
        // Half-open ranges let adjacent regions meet without overlapping. HVF
        // also rejects overlaps, but only with a generic HV_ERROR.
        if self.mappings.iter().any(|existing| {
            mapping.guest_addr < existing.guest_addr + existing.size && existing.guest_addr < end
        }) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "guest memory overlaps an existing HVF mapping",
            ));
        }
        install(mapping)?;
        self.mappings.push(mapping);
        Ok(())
    }

    /// Finds the mapping equal to `region` in all three fields, calls `remove`
    /// and forgets the mapping only if it succeeds.
    pub(super) fn unmap(
        &mut self,
        region: &MemoryRegion,
        remove: impl FnOnce(Mapping) -> io::Result<()>,
    ) -> io::Result<()> {
        let mapping = Mapping::from(region);
        let index = self
            .mappings
            .iter()
            .position(|existing| *existing == mapping)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "HVF memory region is not mapped")
            })?;
        // Keep the record on failure: the guest may still reach the backing.
        remove(mapping)?;
        self.mappings.swap_remove(index);
        Ok(())
    }

    /// Returns the end of a valid mapping, so overlap checks cannot overflow.
    fn validate(&self, mapping: Mapping) -> io::Result<u64> {
        let aligned = |value: u64| value.is_multiple_of(PAGE_SIZE);
        match mapping.guest_addr.checked_add(mapping.size) {
            Some(end)
                if mapping.size != 0
                    && aligned(mapping.size)
                    && aligned(mapping.guest_addr)
                    && aligned(mapping.host_addr as u64)
                    && mapping
                        .host_addr
                        .checked_add(mapping.size as usize)
                        .is_some()
                    && end <= self.ipa_limit =>
            {
                Ok(end)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HVF memory ranges must be nonempty, 16 KiB-aligned, not overflow and fit the IPA size",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::*;

    const PAGE: usize = PAGE_SIZE as usize;
    const HOST: usize = 0x10_0000;

    // The fake HVF calls never dereference these addresses.
    fn range(guest_addr: u64, host_addr: usize, size: usize) -> MemoryRegion {
        let host_addr = NonNull::new(host_addr as *mut u8).unwrap();
        MemoryRegion {
            guest_addr,
            host_addr,
            size,
        }
    }

    fn region(guest_addr: u64) -> MemoryRegion {
        range(guest_addr, HOST, PAGE)
    }

    fn kind(result: io::Result<()>) -> io::ErrorKind {
        result.unwrap_err().kind()
    }

    #[test]
    fn invalid_ranges_never_reach_hvf() {
        // A 36-bit IPA, the default on an M5: guest addresses end at 64 GiB.
        let mut memory = MemoryMap::new(36).unwrap();
        for invalid in [
            range(0, HOST, 0),
            range(0, HOST, PAGE + 4096),
            range(4096, HOST, PAGE),
            range(0, HOST + 4096, PAGE),
            range(0, usize::MAX - PAGE + 1, PAGE),
            range(u64::MAX - PAGE_SIZE + 1, HOST, PAGE),
            range(1 << 36, HOST, PAGE),
        ] {
            let result = memory.map(&invalid, |_| unreachable!());
            assert_eq!(kind(result), io::ErrorKind::InvalidInput);
        }
        // The last page below the IPA limit still fits.
        memory
            .map(&region((1 << 36) - PAGE_SIZE), |_| Ok(()))
            .unwrap();
    }

    #[test]
    fn overlaps_never_reach_hvf_but_neighbours_map() {
        let mut memory = MemoryMap::new(36).unwrap();
        let middle = range(2 * PAGE_SIZE, HOST, 2 * PAGE);
        memory.map(&middle, |_| Ok(())).unwrap();
        for overlapping in [middle, region(3 * PAGE_SIZE)] {
            let result = memory.map(&overlapping, |_| unreachable!());
            assert_eq!(kind(result), io::ErrorKind::AlreadyExists);
        }
        memory.map(&region(PAGE_SIZE), |_| Ok(())).unwrap();
        memory.map(&region(4 * PAGE_SIZE), |_| Ok(())).unwrap();
    }

    #[test]
    fn failed_map_records_nothing() {
        let mut memory = MemoryMap::new(36).unwrap();
        let error = memory.map(&region(0), |_| Err(io::Error::other("HV_ERROR")));
        assert_eq!(error.unwrap_err().to_string(), "HV_ERROR");
        memory
            .map(&region(0), |mapping| {
                assert_eq!(
                    (mapping.guest_addr, mapping.size, mapping.host_addr),
                    (0, PAGE_SIZE, HOST)
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn unmap_requires_all_three_fields_and_keeps_the_record_on_failure() {
        let mut memory = MemoryMap::new(36).unwrap();
        memory.map(&region(0), |_| Ok(())).unwrap();
        for mismatched in [
            region(PAGE_SIZE),
            range(0, HOST, 2 * PAGE),
            range(0, 2 * HOST, PAGE),
        ] {
            let result = memory.unmap(&mismatched, |_| unreachable!());
            assert_eq!(kind(result), io::ErrorKind::NotFound);
        }
        let result = memory.unmap(&region(0), |_| Err(io::Error::other("HV_ERROR")));
        result.unwrap_err();
        let result = memory.map(&region(0), |_| unreachable!());
        assert_eq!(kind(result), io::ErrorKind::AlreadyExists);
        memory.unmap(&region(0), |_| Ok(())).unwrap();
        memory.map(&region(0), |_| Ok(())).unwrap();
    }
}
