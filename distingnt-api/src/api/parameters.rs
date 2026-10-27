//! Reentrant parameter access without borrowing host storage across host calls.
use crate::raw::*;
use core::ffi::CStr;
use core::marker::PhantomData;

/// Pointer-free, borrowed information about a parameter.
pub struct ParameterInfo<'a> {
    pub name: &'a CStr,
    pub min: i16,
    pub max: i16,
    pub default: i16,
    pub unit: u8,
    pub scaling: u8,
}
fn information<'a>(definition: NtParameter) -> ParameterInfo<'a> {
    // SAFETY: only framework-created definitions enter this layer, whose names
    // are immutable static C strings. The result is conservatively borrow-bound.
    ParameterInfo {
        name: unsafe { CStr::from_ptr(definition.name) },
        min: definition.min,
        max: definition.max,
        default: definition.default,
        unit: definition.unit,
        scaling: definition.scaling,
    }
}

/// Bounded parameter values/definitions admitted by the framework, with no raw
/// ABI fields in its public interface.
pub struct ParameterValues<'a> {
    values: &'a [i16],
    definitions: &'a [NtParameter],
}
impl<'a> ParameterValues<'a> {
    #[inline]
    pub(crate) fn new(values: &'a [i16], definitions: &'a [NtParameter]) -> Self {
        Self {
            values,
            definitions,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    #[inline]
    pub fn value(&self, index: usize) -> Option<i16> {
        self.values.get(index).copied()
    }
    #[inline]
    pub fn scaled(&self, index: usize) -> Option<f32> {
        Some(scaled_value(
            self.value(index)?,
            self.definitions.get(index)?.scaling,
        ))
    }
    pub fn definition(&self, index: usize) -> Option<ParameterInfo<'_>> {
        Some(information(*self.definitions.get(index)?))
    }
    /// Check a fixed-size parameter group once. Constant local indices can then
    /// be optimized without repeating the full-instance bounds checks.
    #[inline]
    pub fn group<const N: usize>(&self, start: usize) -> Option<ParameterValues<'_>> {
        let end = start.checked_add(N)?;
        Some(ParameterValues::new(
            self.values.get(start..end)?,
            self.definitions.get(start..end)?,
        ))
    }
}

/// Callbacks permitted to use NT_setParameterFromAudio/updateParameterDefinition.
pub enum Audio {}
/// Any callback using the host's UI-safe setter.
///
/// This context deliberately does not expose audio-only metadata notifications:
/// ```compile_fail
/// use distingnt_api::api::parameters::{Parameters, Ui};
/// fn invalid(parameters: &mut Parameters<'_, Ui>) {
///     parameters.update_definition(0, |_| {});
/// }
/// ```
pub enum Ui {}
/// Parameter reads without a setter capability, usable in any plugin callback.
pub enum ReadOnly {}

/// Mutable numeric constraints; pointer-bearing metadata cannot be changed here.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub min: i16,
    pub max: i16,
}

pub struct Parameters<'callback, Context> {
    algorithm: *mut NtAlgorithm,
    definitions: *mut NtParameter,
    count: usize,
    _lifetime: PhantomData<&'callback NtAlgorithm>,
    _context: PhantomData<Context>,
}

impl<C> Parameters<'_, C> {
    /// # Safety
    /// algorithm is registered and live for this adapter's lifetime. Its v array
    /// and definitions contain exactly the registered plugin's count initialized
    /// entries, with definitions writable.
    /// Keep the adapter within the callback identified by Context (ReadOnly
    /// permits reads in any callback). count must fit the host's u32 index space.
    /// The host must
    /// not destroy/reallocate these regions during synchronous reentry. No Rust
    /// reference to these regions may span a reentrant setter/notification.
    pub(crate) unsafe fn from_raw(
        algorithm: *mut NtAlgorithm,
        definitions: *mut NtParameter,
        count: usize,
    ) -> Self {
        Self {
            algorithm,
            definitions,
            count,
            _lifetime: PhantomData,
            _context: PhantomData,
        }
    }
    #[inline]
    fn index(&self) -> Option<u32> {
        // SAFETY: construction establishes a live registered algorithm.
        u32::try_from(unsafe { NT_algorithmIndex(self.algorithm) }).ok()
    }
    #[inline]
    pub fn value(&self, parameter: usize) -> Option<i16> {
        if parameter >= self.count {
            return None;
        }
        // SAFETY: checked initialized index; copy value without retaining a borrow.
        Some(unsafe { *(*self.algorithm).v.add(parameter) })
    }
    #[inline]
    fn raw_definition(&self, parameter: usize) -> Option<NtParameter> {
        if parameter >= self.count {
            return None;
        }
        // SAFETY: checked initialized index, copied before any host operation.
        Some(unsafe { core::ptr::read(self.definitions.add(parameter)) })
    }
    pub fn definition(&self, parameter: usize) -> Option<ParameterInfo<'_>> {
        Some(information(self.raw_definition(parameter)?))
    }
    // Share scaling in UI/parameter callbacks instead of duplicating its match
    // at every context access. The DSP's ParameterValues remains inlineable.
    #[inline(never)]
    pub fn scaled(&self, parameter: usize) -> Option<f32> {
        Some(scaled_value(
            self.value(parameter)?,
            self.raw_definition(parameter)?.scaling,
        ))
    }
}

impl Parameters<'_, Audio> {
    #[inline]
    pub fn gray(&mut self, parameter: usize, gray: bool) -> Option<()> {
        if parameter >= self.count {
            return None;
        }
        let index = self.index()?;
        // SAFETY: checked parameter and live index; no instance borrow spans the call.
        unsafe {
            NT_setParameterGrayedOut(index, parameter as u32 + NT_parameterOffset(), gray);
        }
        Some(())
    }

    #[inline]
    pub fn set(&mut self, parameter: usize, value: i16) -> Option<()> {
        if parameter >= self.count {
            return None;
        }
        let index = self.index()?;
        // SAFETY: the Audio construction contract permits this callback operation.
        unsafe {
            NT_setParameterFromAudio(index, parameter as u32 + NT_parameterOffset(), value);
        }
        Some(())
    }
    #[inline]
    pub fn update_definition(
        &mut self,
        parameter: usize,
        change: impl FnOnce(&mut Bounds),
    ) -> Option<()> {
        let definition = self.raw_definition(parameter)?;
        let mut bounds = Bounds {
            min: definition.min,
            max: definition.max,
        };
        change(&mut bounds);
        self.publish_definition(parameter, definition, bounds)
    }

    // Share validation/publication across the different caller closure types.
    fn publish_definition(
        &mut self,
        parameter: usize,
        mut definition: NtParameter,
        bounds: Bounds,
    ) -> Option<()> {
        apply_bounds(&mut definition, bounds)?;
        let index = self.index()?;
        // SAFETY: only a local copy was borrowed. Publish before notifying the
        // host; no delayed writeback can overwrite a synchronous nested update.
        unsafe {
            core::ptr::write(self.definitions.add(parameter), definition);
            NT_updateParameterDefinition(index, parameter as u32);
        }
        Some(())
    }
}

impl Parameters<'_, Ui> {
    #[inline]
    pub fn set(&mut self, parameter: usize, value: i16) -> Option<()> {
        if parameter >= self.count {
            return None;
        }
        let index = self.index()?;
        // SAFETY: this callback uses the UI-safe setter, with no live instance borrow.
        unsafe {
            NT_setParameterFromUi(index, parameter as u32 + NT_parameterOffset(), value);
        }
        Some(())
    }
}

#[inline]
pub fn scaled_value(value: i16, scaling: u8) -> f32 {
    value as f32
        / match scaling {
            SCALING_10 => 10.0,
            SCALING_100 => 100.0,
            SCALING_1000 => 1000.0,
            _ => 1.0,
        }
}

// Keep non-generic validation shared across different update closures.
fn apply_bounds(definition: &mut NtParameter, bounds: Bounds) -> Option<()> {
    if bounds.min > bounds.max {
        return None;
    }
    if (UNIT_AUDIO_INPUT..=UNIT_CV_OUTPUT).contains(&definition.unit)
        && (bounds.min < 0 || bounds.max > LAST_BUS as i16)
    {
        return None;
    }
    if definition.unit == UNIT_OUTPUT_MODE && (bounds.min < 0 || bounds.max > 1) {
        return None;
    }
    // Enum-table lengths are opaque in the raw ABI. Never let a safe update
    // expand an enum's index range beyond its currently valid table bounds.
    if definition.unit == UNIT_ENUM && (bounds.min < definition.min || bounds.max > definition.max)
    {
        return None;
    }
    definition.min = bounds.min;
    definition.max = bounds.max;
    Some(())
}
