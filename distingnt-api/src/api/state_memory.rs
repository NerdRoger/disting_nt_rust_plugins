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
    fn in_place_placement_checks_dtc_and_does_not_publish_failed_state() {
        use core::mem::MaybeUninit;
        let mut memory = [0xa5a5a5a5u32; 3];
        let raw = memory.as_mut_ptr().cast::<u8>();
        let mut slot = core::ptr::null_mut::<u32>();
        unsafe {
            for (pointer, bytes) in [(ptr::null_mut(), 4), (raw, 3), (raw.add(1), 4)] {
                assert!(<Dtc as Placement<u32>>::initialize_at(
                    &mut slot,
                    pointer,
                    bytes,
                    |_| panic!("invalid storage admitted")
                )
                .is_none());
            }
            assert!(<Dtc as Placement<u32>>::initialize_at(&mut slot, raw, 4, |_| None).is_none());
            assert!(slot.is_null());
            assert_eq!(memory, [0xa5a5a5a5; 3]);
            <Dtc as Placement<u32>>::initialize_at(&mut slot, raw.add(4), 4, |destination| {
                destination.write(99);
                Some(())
            })
            .unwrap();
            assert_eq!(slot, memory.as_mut_ptr().add(1));
            assert_eq!(memory, [0xa5a5a5a5, 99, 0xa5a5a5a5]);
            let mut sram = MaybeUninit::<u32>::uninit();
            <Sram as Placement<u32>>::initialize_at(
                sram.as_mut_ptr(),
                ptr::null_mut(),
                0,
                |destination| {
                    destination.write(123);
                    Some(())
                },
            )
            .unwrap();
            assert_eq!(sram.assume_init(), 123);
        }
    }
}

mod storage {
    use super::*;
    pub trait Placement<S: Copy> {
        type Slot: Copy;
        fn dtc_size() -> usize;
        /// Caller supplies an exclusive aligned Slot destination. The callback
        /// must initialize S before returning Some; failure is never published.
        unsafe fn initialize_at(
            slot: *mut Self::Slot,
            memory: *mut u8,
            bytes: usize,
            initialize: impl FnOnce(*mut S) -> Option<()>,
        ) -> Option<()>;
        unsafe fn pointer(slot: *mut Self::Slot) -> *mut S;
    }
    impl<S: Copy> Placement<S> for Sram {
        type Slot = S;
        fn dtc_size() -> usize {
            0
        }
        #[inline(always)]
        unsafe fn initialize_at(
            slot: *mut S,
            _: *mut u8,
            _: usize,
            initialize: impl FnOnce(*mut S) -> Option<()>,
        ) -> Option<()> {
            initialize(slot)
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
        #[inline(always)]
        unsafe fn initialize_at(
            slot: *mut *mut S,
            memory: *mut u8,
            bytes: usize,
            initialize: impl FnOnce(*mut S) -> Option<()>,
        ) -> Option<()> {
            if memory.is_null() || bytes < size_of::<S>() || memory as usize % align_of::<S>() != 0
            {
                return None;
            }
            let state = memory.cast::<S>();
            initialize(state)?;
            // SAFETY: exclusive aligned slot; callback initialized state.
            unsafe {
                ptr::write(slot, state);
            }
            Some(())
        }
        unsafe fn pointer(slot: *mut *mut S) -> *mut S {
            // SAFETY: registration initialized this slot with a checked live DTC pointer.
            unsafe { *slot }
        }
    }
}
