//! Drawing capability, valid only for an active host draw callback.
use crate::raw::*;
use core::ffi::CStr;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum Shape {
    Point = SHAPE_POINT,
    Line = SHAPE_LINE,
    Box = SHAPE_BOX,
    Rectangle = SHAPE_RECTANGLE,
    Circle = SHAPE_CIRCLE,
}
impl Shape {
    pub fn from_raw(value: u8) -> Option<Self> {
        match value {
            SHAPE_POINT => Some(Self::Point),
            SHAPE_LINE => Some(Self::Line),
            SHAPE_BOX => Some(Self::Box),
            SHAPE_RECTANGLE => Some(Self::Rectangle),
            SHAPE_CIRCLE => Some(Self::Circle),
            _ => None,
        }
    }
}
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum Alignment {
    Left = TEXT_LEFT,
    Centre = TEXT_CENTRE,
    Right = TEXT_RIGHT,
}
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum TextSize {
    Tiny = TEXT_TINY,
    Normal = TEXT_NORMAL,
    Large = TEXT_LARGE,
}

/// A framework-provided canvas cannot be retained beyond its draw callback.
/// ```compile_fail
/// use distingnt_api::api::drawing::Canvas;
/// fn retain(canvas: Canvas<'_>) -> Canvas<'static> { canvas }
/// ```
pub struct Canvas<'draw> {
    _context: core::marker::PhantomData<*mut ()>,
    _lifetime: core::marker::PhantomData<&'draw mut ()>,
}
impl Canvas<'_> {
    /// # Safety
    /// Create/use only inside a host draw callback. Do not let the capability
    /// escape the callback or use it concurrently with another drawing context.
    pub(crate) unsafe fn new() -> Self {
        Self {
            _context: core::marker::PhantomData,
            _lifetime: core::marker::PhantomData,
        }
    }
    #[inline]
    pub fn shape(&mut self, kind: Shape, x0: i32, y0: i32, x1: i32, y1: i32, colour: i32) {
        // SAFETY: construction establishes draw context; all arguments are values.
        unsafe {
            NT_drawShapeI(kind as u8, x0, y0, x1, y1, colour);
        }
    }
    #[inline]
    pub fn shape_f(&mut self, kind: Shape, x0: f32, y0: f32, x1: f32, y1: f32, colour: f32) {
        // SAFETY: construction establishes the active draw context.
        unsafe {
            NT_drawShapeF(kind as u8, x0, y0, x1, y1, colour);
        }
    }
    #[inline]
    pub fn text(
        &mut self,
        x: i32,
        y: i32,
        text: &CStr,
        colour: i32,
        align: Alignment,
        size: TextSize,
    ) {
        // SAFETY: context is active and the host consumes the C string immediately.
        unsafe {
            NT_drawText(x, y, text.as_ptr(), colour, align as u8, size as u8);
        }
    }
}
