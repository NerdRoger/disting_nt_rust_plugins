//! Background adapters under the firmware teardown contract documented in README.
//! Firmware must stop/drain jobs AND large-SysEx callbacks before freeing their
//! instance memory or unloading plugin code. No callback may survive teardown.
//!
//! Example: a preallocated payload shared by normal callbacks and a worker.
//! ```no_run
//! #![forbid(unsafe_code)]
//! use distingnt_api::api::{plugin::{Plugin,Configuration,RegionInitializer,Metadata,
//!     SharedResources,Context},parameters::Audio,state_memory::Sram,
//!     background::{SysExChunk,Error}};
//! #[derive(Clone,Copy)]
//! struct Data { bytes:[u8;128], sent:bool }
//! struct Example;
//! impl Plugin for Example {
//!     type State = ();
//!     type Region = ();
//!     type StateMemory = Sram;
//!     type ExtraRegions = SharedResources<Data>;
//!     const GUID:u32 = 0x74657374;
//!     const NAME: &'static core::ffi::CStr = c"Background example";
//!     const DESCRIPTION: &'static core::ffi::CStr = c"";
//!     const TAGS:u32 = 0;
//!     const SPECIFICATIONS: &'static [distingnt_api::api::plugin::Specification] = &[];
//!     const AUDIO_WORKER_STACK_SIZE:u32 = 1024;
//!     fn configuration(_: &[i32]) -> Option<Configuration> {
//!         Some(Configuration { parameters:0,pages:0,page_indices:0,regions:0 })
//!     }
//!     fn metadata(_: &[i32],_: &mut Metadata<'_>) -> Option<()> { Some(()) }
//!     fn initialize_state(p:&mut RegionInitializer<'_,()>,_: &[i32],_: u32) -> Option<()> {
//!         p.set(()); Some(())
//!     }
//!     fn initialize_region(p:&mut RegionInitializer<'_,()>,_: &[i32],_: usize) -> Option<()> {
//!         p.set(()); Some(())
//!     }
//!     fn initialize_shared_resources(p:&mut RegionInitializer<'_,Data>,_: &[i32]) -> Option<()> {
//!         distingnt_api::initialize_region!(p,Data { sent:false },arrays { bytes: |_| 0 });
//!         Some(())
//!     }
//!     fn audio_worker(data:&mut Data,tag:u32) { data.bytes.fill((tag & 127) as u8); }
//!     fn large_sysex_chunk(data:&mut Data) -> SysExChunk<'_> {
//!         data.sent = true;
//!         SysExChunk { bytes:&data.bytes, end:true }
//!     }
//! }
//! fn queue(context:&mut Context<'_,(),(),Audio,Sram,SharedResources<Data>>) -> Result<(),Error> {
//!     context.with_shared_resources(|data| data.sent=false)?;
//!     context.submit_audio_worker(42)
//! }
//! ```
use super::plugin::RegionInitializer;
use crate::raw::{
    LargeSysExComplete, LargeSysExFetch, NT_sendLargeMidiSysEx, NtSendLargeSysExRequest,
};
use core::{
    cell::UnsafeCell,
    mem::{align_of, size_of},
    ptr, slice,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Disabled,
    Busy,
    QueueFull,
    NoAlgorithm,
}

/// Opt-in instance DRAM split into up to COUNT typed buffers. Normal callbacks
/// and background workers can borrow different buffers without waiting.
pub struct DramBuffers<T, const COUNT: usize> {
    elements: *mut T,
    ends: [usize; COUNT],
    count: usize,
    busy: AtomicU32,
}

impl<T, const COUNT: usize> DramBuffers<T, COUNT> {
    /// Borrow a buffer for the duration of the closure. Returns None if the
    /// index is out of range or a callback already holds that buffer.
    pub fn with<R>(&self, index: usize, access: impl FnOnce(&mut [T]) -> R) -> Option<R> {
        if index >= self.count {
            return None;
        }
        let end = *self.ends.get(index)?;
        let start = index
            .checked_sub(1)
            .and_then(|i| self.ends.get(i))
            .copied()
            .unwrap_or(0);
        let bit = 1 << index;
        if self.busy.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
            return None;
        }
        struct Release<'a>(&'a AtomicU32, u32);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.fetch_and(!self.1, Ordering::Release);
            }
        }
        let _release = Release(&self.busy, bit);
        // The lock covers exactly this initialized, disjoint slice. The slice
        // cannot escape the closure and release publishes all buffer writes.
        Some(access(unsafe {
            slice::from_raw_parts_mut(self.elements.add(start), end - start)
        }))
    }
}

// Every access to the disjoint spans is mediated by the reservation above.
unsafe impl<T: Send, const COUNT: usize> Sync for DramBuffers<T, COUNT> {}

mod dram_storage {
    pub trait Storage: Sized {
        const ENABLED: bool;
        const MAX_COUNT: usize;
        fn bytes(count: usize, length: impl FnMut(usize) -> usize) -> Option<u32>;
        /// # Safety
        /// The host supplies an exclusive instance allocation of `available`
        /// bytes that will remain live until all callbacks finish.
        unsafe fn initialize(
            memory: *mut u8,
            available: u32,
            count: usize,
            length: impl FnMut(usize) -> usize,
        ) -> Option<Self>;
    }
}
pub(crate) use dram_storage::Storage as DramStorage;

/// Selects an optional DRAM allocation. Use `()` for no buffers or
/// `DramBuffers<T, COUNT>` for up to 32 independently accessible buffers.
pub trait DramBufferPolicy: dram_storage::Storage + 'static {}
impl DramBufferPolicy for () {}
impl<T: Copy + Default + Send + 'static, const COUNT: usize> DramBufferPolicy
    for DramBuffers<T, COUNT>
{
}

impl DramStorage for () {
    const ENABLED: bool = false;
    const MAX_COUNT: usize = 0;
    fn bytes(_: usize, _: impl FnMut(usize) -> usize) -> Option<u32> {
        Some(0)
    }
    unsafe fn initialize(
        _: *mut u8,
        _: u32,
        _: usize,
        _: impl FnMut(usize) -> usize,
    ) -> Option<Self> {
        Some(())
    }
}

impl<T, const COUNT: usize> DramBuffers<T, COUNT> {
    fn layout(
        count: usize,
        mut length: impl FnMut(usize) -> usize,
    ) -> Option<([usize; COUNT], usize, u32)> {
        if !(1..=COUNT).contains(&count) || COUNT > u32::BITS as usize || size_of::<T>() == 0 {
            return None;
        }
        let mut ends = [0; COUNT];
        let mut total = 0usize;
        for (index, end) in ends.iter_mut().take(count).enumerate() {
            let length = length(index);
            if length == 0 {
                return None;
            }
            total = total.checked_add(length)?;
            *end = total;
        }
        let bytes = core::alloc::Layout::array::<T>(total).ok()?.size();
        Some((ends, total, u32::try_from(bytes).ok()?))
    }
}

impl<T: Copy + Default + Send, const COUNT: usize> DramStorage for DramBuffers<T, COUNT> {
    const ENABLED: bool = true;
    const MAX_COUNT: usize = COUNT;

    fn bytes(count: usize, length: impl FnMut(usize) -> usize) -> Option<u32> {
        Some(Self::layout(count, length)?.2)
    }

    unsafe fn initialize(
        memory: *mut u8,
        available: u32,
        count: usize,
        length: impl FnMut(usize) -> usize,
    ) -> Option<Self> {
        let (ends, total, bytes) = Self::layout(count, length)?;
        if bytes > available || memory.is_null() || memory as usize % align_of::<T>() != 0 {
            return None;
        }
        let elements = memory.cast::<T>();
        let initial = T::default();
        for index in 0..total {
            // The checked allocation is aligned and exclusive. Write directly
            // into DRAM, including for types whose default is not all-zero bits.
            unsafe { elements.add(index).write(initial) };
        }
        Some(Self {
            elements,
            ends,
            count,
            busy: AtomicU32::new(0),
        })
    }
}

/// Borrowed data remains reserved until the host consumes it. Firmware consumes
/// each chunk before the next fetch; completion runs after the final data is sent.
pub struct SysExChunk<'a> {
    pub bytes: &'a [u8],
    pub end: bool,
}

// This type is never returned to a plugin and is not Copy. Keeping the atomic
// outside the Copy payload prevents callers copying away the ownership gate.
pub(crate) struct SharedSlot<T: Copy + Send> {
    busy: AtomicBool,
    value: UnsafeCell<T>,
    request: UnsafeCell<NtSendLargeSysExRequest>,
}
// All accesses to the payload/request require exclusive reservation. The payload
// may move between execution contexts (Send); Sync is not required of it.
unsafe impl<T: Copy + Send> Sync for SharedSlot<T> {}
impl<T: Copy + Send> SharedSlot<T> {
    pub(crate) unsafe fn initialize(
        destination: *mut Self,
        initialize: impl FnOnce(&mut RegionInitializer<'_, T>) -> Option<()>,
        fetch: LargeSysExFetch,
        complete: LargeSysExComplete,
    ) -> Option<()> {
        // UnsafeCell has the same representation/alignment as T. Destination is
        // the exclusive aligned slot admitted by the checked framework layout.
        let value = unsafe { ptr::addr_of_mut!((*destination).value).cast::<T>() };
        let mut writer = unsafe { RegionInitializer::new(value) };
        initialize(&mut writer)?;
        if !writer.initialized {
            return None;
        }
        unsafe {
            ptr::addr_of_mut!((*destination).busy).write(AtomicBool::new(false));
            ptr::addr_of_mut!((*destination).request).write(UnsafeCell::new(
                NtSendLargeSysExRequest {
                    destination: 0,
                    fetch_callback: Some(fetch),
                    complete_callback: Some(complete),
                    callback_data: destination.cast(),
                },
            ));
        }
        Some(())
    }
    pub(crate) fn reserve(&self) -> Result<(), Error> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| Error::Busy)
    }
    pub(crate) fn release(&self) {
        self.busy.store(false, Ordering::Release);
    }
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, Error> {
        self.reserve()?;
        // Private guard cannot be forgotten by plugin code; releases on unwind.
        let _release = Release(self);
        Ok(f(unsafe { &mut *self.value.get() }))
    }
    /// Caller owns the reservation; firmware may not concurrently fetch/complete.
    pub(crate) unsafe fn reserved_value(&self) -> &mut T {
        // Pair with reservation's release before the host hands off execution.
        let reserved = self.busy.load(Ordering::Acquire);
        debug_assert!(reserved);
        unsafe { &mut *self.value.get() }
    }
    pub(crate) unsafe fn run(&self, f: impl FnOnce(&mut T)) {
        let _release = Release(self);
        f(unsafe { self.reserved_value() });
    }
    pub(crate) fn send(&self, destination: u32) -> Result<(), Error> {
        self.reserve()?;
        let request = self.request.get();
        // No payload/reference is borrowed across this potentially synchronous
        // host call. Only completion releases the reservation after consumption.
        unsafe {
            (*request).destination = destination;
            NT_sendLargeMidiSysEx(request);
        }
        Ok(())
    }
}
struct Release<'a, T: Copy + Send>(&'a SharedSlot<T>);
impl<T: Copy + Send> Drop for Release<'_, T> {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::mem::MaybeUninit;
    unsafe extern "C" fn fetch(
        _: *mut core::ffi::c_void,
        _: *mut *const u8,
        _: *mut u32,
        _: *mut bool,
    ) {
    }
    unsafe extern "C" fn complete(_: *mut core::ffi::c_void) {}

    #[test]
    fn a_worker_buffer_excludes_only_its_own_span() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Entry(u16);
        impl Default for Entry {
            fn default() -> Self {
                Self(13)
            }
        }
        type Buffers = DramBuffers<Entry, 4>;
        let mut dram = [Entry(255); 13];
        let lengths = |i| [2, 4, 6][i];
        assert_eq!(Buffers::bytes(3, lengths), Some(24));
        assert_eq!(Buffers::bytes(5, lengths), None);
        assert_eq!(DramBuffers::<Entry, 33>::bytes(3, |_| 1), None);
        assert_eq!(Buffers::bytes(0, lengths), None);
        assert_eq!(Buffers::bytes(2, |_| 0), None);
        let memory = dram.as_mut_ptr().cast::<u8>();
        assert!(unsafe { Buffers::initialize(memory, 23, 3, lengths) }.is_none());
        let buffers = unsafe { Buffers::initialize(memory, 24, 3, lengths) }.unwrap();
        let entered = std::sync::Barrier::new(2);
        let finish = std::sync::Barrier::new(2);
        std::thread::scope(|threads| {
            threads.spawn(|| {
                buffers
                    .with(1, |samples| {
                        assert_eq!(samples, [Entry(13); 4]);
                        samples.fill(Entry(3));
                        entered.wait();
                        finish.wait();
                    })
                    .unwrap();
            });
            entered.wait();
            assert_eq!(buffers.with(1, |_| ()), None);
            assert_eq!(buffers.with(3, |_| ()), None);
            buffers.with(0, |samples| samples.fill(Entry(5))).unwrap();
            assert_eq!(buffers.with(2, |samples| samples[0]), Some(Entry(13)));
            finish.wait();
        });
        assert_eq!(buffers.with(0, |samples| samples[0]), Some(Entry(5)));
        assert_eq!(buffers.with(1, |samples| samples[0]), Some(Entry(3)));
        assert_eq!(dram[12], Entry(255));
    }

    #[test]
    fn reservation_excludes_another_thread_and_publishes_worker_writes() {
        let mut memory = MaybeUninit::<SharedSlot<u32>>::uninit();
        unsafe {
            SharedSlot::initialize(
                memory.as_mut_ptr(),
                |p| {
                    p.set(10);
                    Some(())
                },
                fetch,
                complete,
            )
            .unwrap();
        }
        let slot = unsafe { memory.assume_init_ref() };
        let entered = std::sync::Barrier::new(2);
        let finish = std::sync::Barrier::new(2);
        slot.reserve().unwrap();
        std::thread::scope(|threads| {
            threads.spawn(|| unsafe {
                slot.run(|value| {
                    *value = 99;
                    entered.wait();
                    finish.wait();
                });
            });
            entered.wait();
            assert_eq!(slot.with(|_| ()), Err(Error::Busy));
            assert_eq!(slot.reserve(), Err(Error::Busy));
            finish.wait();
        });
        assert_eq!(slot.with(|v| *v), Ok(99));
    }
    #[test]
    fn closure_unwind_releases_the_private_guard() {
        let mut memory = MaybeUninit::<SharedSlot<u32>>::uninit();
        unsafe {
            SharedSlot::initialize(
                memory.as_mut_ptr(),
                |p| {
                    p.set(10);
                    Some(())
                },
                fetch,
                complete,
            )
            .unwrap();
        }
        let slot = unsafe { memory.assume_init_ref() };
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = slot.with(|_| panic!("fixture unwind"));
        }))
        .is_err());
        assert_eq!(slot.with(|v| *v), Ok(10));
    }
}
