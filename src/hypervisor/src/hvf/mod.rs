// Copyright 2026 BoxLite Contributors
// SPDX-License-Identifier: Apache-2.0

//! Hypervisor.framework operations on macOS arm64.

mod ffi;
mod memory;
mod syndrome;
mod vm;

pub use vm::HvfVm;
