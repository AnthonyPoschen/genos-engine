//! Host-visible bytes for one recording of a reused command buffer.
//!
//! The slot is reset after that buffer's fence has retired. The allocation
//! stays, so a steady frame does not call `vkAllocateMemory` again.

use super::Buffer;

pub(super) struct Staged {
    pub(super) buffer: Buffer,
    pub(super) mapped: *mut u8,
}

impl Staged {
    pub(super) fn empty() -> Self {
        Self {
            buffer: Buffer::empty(),
            mapped: std::ptr::null_mut(),
        }
    }
}

pub(super) struct Slot {
    pub(super) main: Staged,
    pub(super) cursor: u64,
    pub(super) filled: u64,
    pub(super) overflow: Vec<Staged>,
}

impl Slot {
    fn empty() -> Self {
        Self {
            main: Staged::empty(),
            cursor: 0,
            filled: 0,
            overflow: Vec::new(),
        }
    }
}

pub(super) struct StageSet {
    pub(super) slots: [Slot; 6],
}

impl StageSet {
    pub(super) fn empty() -> Self {
        Self {
            slots: [Slot::empty(), Slot::empty(), Slot::empty(), Slot::empty(), Slot::empty(), Slot::empty()],
        }
    }
}
