//! Sealed, statically selected state placement; no per-access dispatch branch.
use core::{
    mem::{align_of, size_of},
    ptr,
};

pub struct Sram;
pub struct Dtc;
pub trait StateMemory<S: Copy>: storage::Placement<S> {}
impl<S: Copy> StateMemory<S> for Sram {}
impl<S: Copy> StateMemory<S> for Dtc {}
pub(crate) use storage::Placement;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dtc_placement_checks_capacity_and_alignment() {
        let mut memory = [0xa5a5_a5a5u32; 4];
        let raw = memory.as_mut_ptr().cast::<u8>();
        unsafe {
            assert!(<Dtc as Placement<u32>>::initialize(42, ptr::null_mut(), 4).is_none());
            assert!(<Dtc as Placement<u32>>::initialize(42, raw, 3).is_none());
            assert!(<Dtc as Placement<u32>>::initialize(42, raw.add(1), 4).is_none());
            assert_eq!(memory[0], 0xa5a5_a5a5);
            let mut slot = <Dtc as Placement<u32>>::initialize(42, raw, 4).unwrap();
            assert_eq!(
                <Dtc as Placement<u32>>::pointer(&mut slot),
                memory.as_mut_ptr()
            );
            assert_eq!(memory[0], 42);
        }
    }
}

mod storage {
    use super::*;
    pub trait Placement<S: Copy> {
        type Slot: Copy;
        fn dtc_size() -> usize;
        unsafe fn initialize(value: S, memory: *mut u8, bytes: usize) -> Option<Self::Slot>;
        unsafe fn pointer(slot: *mut Self::Slot) -> *mut S;
    }
    impl<S: Copy> Placement<S> for Sram {
        type Slot = S;
        fn dtc_size() -> usize {
            0
        }
        unsafe fn initialize(value: S, _: *mut u8, _: usize) -> Option<S> {
            Some(value)
        }
        unsafe fn pointer(slot: *mut S) -> *mut S {
            slot
        }
    }
    impl<S: Copy> Placement<S> for Dtc {
        type Slot = *mut S;
        fn dtc_size() -> usize {
            size_of::<S>()
        }
        unsafe fn initialize(value: S, memory: *mut u8, bytes: usize) -> Option<*mut S> {
            if memory.is_null() || bytes < size_of::<S>() || memory as usize % align_of::<S>() != 0
            {
                return None;
            }
            let state = memory.cast::<S>();
            // SAFETY: the caller provides exclusive live DTC; size/alignment checked.
            unsafe {
                ptr::write(state, value);
            }
            Some(state)
        }
        unsafe fn pointer(slot: *mut *mut S) -> *mut S {
            // SAFETY: registration initialized this slot with a checked live DTC pointer.
            unsafe { *slot }
        }
    }
}
