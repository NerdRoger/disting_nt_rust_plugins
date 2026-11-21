//! Callback-scoped access to immutable DRAM initialized at factory load.
use core::{
    marker::PhantomData,
    mem::{align_of, size_of},
    ptr, slice,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

mod sealed {
    pub trait Element {}
}

/// Numeric elements whose memory representation accepts any bit pattern.
pub trait StaticDramElement: Copy + Sync + sealed::Element {}

macro_rules! elements {
    ($($element:ty),+) => {
        $(
            impl sealed::Element for $element {}
            impl StaticDramElement for $element {}
        )+
    };
}
elements!(f32, f64, u8, i8, u16, i16, u32, i32, u64, i64);

fn element_count<T>(bytes: &[u8]) -> Option<usize> {
    (bytes.as_ptr() as usize % align_of::<T>() == 0 && bytes.len() % size_of::<T>() == 0)
        .then_some(bytes.len() / size_of::<T>())
}

pub(crate) fn elements_mut<T: StaticDramElement>(bytes: &mut [u8]) -> Option<&mut [T]> {
    if bytes.is_empty() {
        return Some(&mut []);
    }
    let count = element_count::<T>(bytes)?;
    // SAFETY: the layout is checked, all bit patterns are valid for T, and
    // the returned lifetime retains exclusive access to these bytes.
    Some(unsafe { slice::from_raw_parts_mut(bytes.as_mut_ptr().cast(), count) })
}

/// Immutable initialized DRAM, borrowed for one callback.
///
/// ```compile_fail
/// use distingnt_api::api::plugin::StaticDramView;
/// fn retain(view: StaticDramView<'_>) -> &'static [f32] {
///     view.elements::<f32>().unwrap()
/// }
/// ```
#[derive(Clone, Copy)]
pub struct StaticDramView<'a> {
    bytes: &'a [u8],
}

impl<'a> StaticDramView<'a> {
    /// Borrow the entire allocation as elements if its length and alignment fit.
    pub fn elements<T: StaticDramElement>(self) -> Option<&'a [T]> {
        if self.bytes.is_empty() {
            return Some(&[]);
        }
        let count = element_count::<T>(self.bytes)?;
        // SAFETY: same checked layout and representation as elements_mut.
        // The firmware allocation remains immutable for this callback.
        Some(unsafe { slice::from_raw_parts(self.bytes.as_ptr().cast(), count) })
    }
}

/// Factory-owned descriptor, keyed by the plugin type and used by registration.
/// Safe plugin code can neither publish arbitrary pointers nor read after teardown.
pub struct SharedStaticDram<P> {
    pointer: AtomicPtr<u8>,
    length: AtomicUsize,
    _plugin: PhantomData<fn() -> P>,
}

impl<P> SharedStaticDram<P> {
    pub const fn new() -> Self {
        Self {
            pointer: AtomicPtr::new(ptr::null_mut()),
            length: AtomicUsize::new(0),
            _plugin: PhantomData,
        }
    }

    pub(crate) fn clear(&self) {
        self.pointer.store(ptr::null_mut(), Ordering::Release);
    }

    /// The host must retain the initialized allocation unchanged until all
    /// callbacks for this loaded factory finish, including queued workers.
    pub(crate) unsafe fn publish(&self, pointer: *mut u8, length: usize) {
        self.length.store(length, Ordering::Relaxed);
        self.pointer.store(pointer, Ordering::Release);
    }

    /// Caller is a live callback for the registered factory. Reinitialization
    /// and teardown must be serialized with callbacks by the host.
    pub(crate) unsafe fn with_view<R>(&self, read: impl FnOnce(StaticDramView<'_>) -> R) -> R {
        let pointer = self.pointer.load(Ordering::Acquire);
        let bytes = if pointer.is_null() {
            &[]
        } else {
            let length = self.length.load(Ordering::Relaxed);
            // SAFETY: initialise published this live, immutable allocation.
            unsafe { slice::from_raw_parts(pointer, length) }
        };
        read(StaticDramView { bytes })
    }
}

impl<P> Default for SharedStaticDram<P> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialized_elements_can_be_read_only_within_a_view() {
        let resource = SharedStaticDram::<()>::new();
        let mut storage = [0u64; 2];
        let pointer = storage.as_mut_ptr().cast::<u8>();
        let bytes = unsafe { slice::from_raw_parts_mut(pointer, 16) };
        assert!(elements_mut::<f32>(&mut bytes[1..]).is_none());
        elements_mut::<f32>(bytes)
            .unwrap()
            .copy_from_slice(&[0.25, 2.0, -3.0, 4.5]);
        unsafe { resource.publish(pointer, 16) };
        unsafe {
            resource.with_view(|view| {
                assert_eq!(view.elements::<f32>(), Some(&[0.25, 2.0, -3.0, 4.5][..]))
            });
        }
        resource.clear();
        unsafe { resource.with_view(|view| assert_eq!(view.elements::<f32>(), Some(&[][..]))) };
    }
}
