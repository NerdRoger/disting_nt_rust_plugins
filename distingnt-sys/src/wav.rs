//! Sample, wavetable, stream and MIDI-folder API. Asynchronous requests and
//! their buffers must remain valid until their completion callback is invoked.
use crate::NtFrame;
use core::ffi::{c_char, c_void};
pub const WAV_NO_PROGRESS: u8 = 0;
pub const WAV_PROGRESS: u8 = 1;
pub const WAV_MONO: u8 = 0;
pub const WAV_STEREO: u8 = 1;
pub const WAV_BITS_8: u8 = 0;
pub const WAV_BITS_16: u8 = 1;
pub const WAV_BITS_24: u8 = 2;
pub const WAV_BITS_32: u8 = 3;
pub const RR_MODE_SEQUENTIAL: u8 = 0;
pub const RR_MODE_RANDOM: u8 = 1;
pub const RR_MODE_RANDOM_2: u8 = 2;
pub const RR_MODE_RANDOM_3: u8 = 3;
pub const NUM_RR_MODES: u8 = 4;
pub type NtStream = *mut c_void;
pub type ReadCallback = unsafe extern "C" fn(*mut c_void, bool);
pub type CompletionCallback = unsafe extern "C" fn(*mut c_void);
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavFolderInfo {
    pub name: *const c_char,
    pub num_sample_files: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavInfo {
    pub name: *const c_char,
    pub num_frames: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavRequest {
    pub folder: u32,
    pub sample: u32,
    pub dst: *mut c_void,
    pub num_frames: u32,
    pub start_offset: u32,
    pub channels: u8,
    pub bits: u8,
    pub progress: u8,
    pub callback: Option<ReadCallback>,
    pub callback_data: *mut c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavetableInfo {
    pub name: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavetableRequest {
    pub index: u32,
    pub table: *mut i16,
    pub table_size: u32,
    pub callback: Option<CompletionCallback>,
    pub callback_data: *mut c_void,
    pub error: bool,
    pub using_mip_maps: bool,
    pub num_waves: u32,
    pub wave_length: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtWavetableEvaluation {
    pub phase: f32,
    pub offset: f32,
    pub frequency: f32,
    pub mipmap_size: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtMidiFileFolderInfo {
    pub name: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtStreamOpenData {
    pub stream_buffer: *mut c_void,
    pub folder: u32,
    pub sample: u32,
    pub velocity: f32,
    pub start_offset: u32,
    pub reverse: bool,
    pub rr_mode: u8,
}
unsafe extern "C" {
    pub fn NT_isSdCardMounted() -> bool;
    pub fn NT_getNumSampleFolders() -> u32;
    pub fn NT_getSampleFolderInfo(folder: u32, info: *mut NtWavFolderInfo);
    pub fn NT_getSampleFileInfo(folder: u32, sample: u32, info: *mut NtWavInfo);
    pub fn NT_readSampleFrames(request: *const NtWavRequest) -> bool;
    pub fn NT_getNumWavetables() -> u32;
    pub fn NT_getWavetableInfo(index: u32, info: *mut NtWavetableInfo);
    pub fn NT_readWavetable(request: *mut NtWavetableRequest) -> bool;
    pub fn NT_evaluateWavetable(
        request: *const NtWavetableRequest,
        eval: *mut NtWavetableEvaluation,
    ) -> f32;
    pub fn NT_streamOpen(stream: NtStream, data: *const NtStreamOpenData) -> bool;
    pub fn NT_streamRender(
        stream: NtStream,
        render_buffer: *mut NtFrame,
        num_frames: u32,
        speed: f32,
    ) -> u32;
    pub fn NT_getNumMidiFileFolders() -> u32;
    pub fn NT_getMidiFileFolderInfo(folder: u32, info: *mut NtMidiFileFolderInfo);
}
