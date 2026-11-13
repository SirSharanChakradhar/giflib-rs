//! C-compatible types and constants, matching `gif_lib.h` byte for byte.

#![allow(non_snake_case, non_camel_case_types)]

use core::ffi::{c_int, c_void};
use core::mem::{offset_of, size_of};

pub type GifByteType = u8;
pub type GifPixelType = u8;
pub type GifWord = c_int;
pub type GifRecordType = c_int;

pub const GIF_ERROR: c_int = 0;
pub const GIF_OK: c_int = 1;

pub const GIF87_STAMP: &core::ffi::CStr = c"GIF87a";
pub const GIF89_STAMP: &core::ffi::CStr = c"GIF89a";

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct GifColorType {
    pub Red: GifByteType,
    pub Green: GifByteType,
    pub Blue: GifByteType,
}

#[repr(C)]
pub struct ColorMapObject {
    pub ColorCount: c_int,
    pub BitsPerPixel: c_int,
    pub SortFlag: bool,
    /// On the malloc(3) heap.
    pub Colors: *mut GifColorType,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GifImageDesc {
    pub Left: GifWord,
    pub Top: GifWord,
    pub Width: GifWord,
    pub Height: GifWord,
    pub Interlace: bool,
    pub ColorMap: *mut ColorMapObject,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtensionBlock {
    pub ByteCount: c_int,
    /// On the malloc(3) heap.
    pub Bytes: *mut GifByteType,
    pub Function: c_int,
}

pub const CONTINUE_EXT_FUNC_CODE: c_int = 0x00;
pub const COMMENT_EXT_FUNC_CODE: c_int = 0xfe;
pub const GRAPHICS_EXT_FUNC_CODE: c_int = 0xf9;
pub const PLAINTEXT_EXT_FUNC_CODE: c_int = 0x01;
pub const APPLICATION_EXT_FUNC_CODE: c_int = 0xff;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SavedImage {
    pub ImageDesc: GifImageDesc,
    /// On the malloc(3) heap.
    pub RasterBits: *mut GifByteType,
    pub ExtensionBlockCount: c_int,
    pub ExtensionBlocks: *mut ExtensionBlock,
}

#[repr(C)]
pub struct GifFileType {
    pub SWidth: GifWord,
    pub SHeight: GifWord,
    pub SColorResolution: GifWord,
    pub SBackGroundColor: GifWord,
    pub AspectByte: GifByteType,
    pub SColorMap: *mut ColorMapObject,
    pub ImageCount: c_int,
    pub Image: GifImageDesc,
    pub SavedImages: *mut SavedImage,
    pub ExtensionBlockCount: c_int,
    pub ExtensionBlocks: *mut ExtensionBlock,
    pub Error: c_int,
    pub UserData: *mut c_void,
    pub Private: *mut c_void,
}

pub const UNDEFINED_RECORD_TYPE: GifRecordType = 0;
pub const SCREEN_DESC_RECORD_TYPE: GifRecordType = 1;
pub const IMAGE_DESC_RECORD_TYPE: GifRecordType = 2;
pub const EXTENSION_RECORD_TYPE: GifRecordType = 3;
pub const TERMINATE_RECORD_TYPE: GifRecordType = 4;

pub type InputFunc =
    Option<unsafe extern "C" fn(*mut GifFileType, *mut GifByteType, c_int) -> c_int>;
pub type OutputFunc =
    Option<unsafe extern "C" fn(*mut GifFileType, *const GifByteType, c_int) -> c_int>;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct GraphicsControlBlock {
    pub DisposalMode: c_int,
    pub UserInputFlag: bool,
    pub DelayTime: c_int,
    pub TransparentColor: c_int,
}

pub const DISPOSAL_UNSPECIFIED: c_int = 0;
pub const NO_TRANSPARENT_COLOR: c_int = -1;

// Encoder errors.
pub const E_GIF_SUCCEEDED: c_int = 0;
pub const E_GIF_ERR_OPEN_FAILED: c_int = 201;
pub const E_GIF_ERR_WRITE_FAILED: c_int = 202;
pub const E_GIF_ERR_HAS_SCRN_DSCR: c_int = 203;
pub const E_GIF_ERR_HAS_IMAG_DSCR: c_int = 204;
pub const E_GIF_ERR_NO_COLOR_MAP: c_int = 205;
pub const E_GIF_ERR_DATA_TOO_BIG: c_int = 206;
pub const E_GIF_ERR_NOT_ENOUGH_MEM: c_int = 207;
pub const E_GIF_ERR_DISK_IS_FULL: c_int = 208;
pub const E_GIF_ERR_CLOSE_FAILED: c_int = 209;
pub const E_GIF_ERR_NOT_WRITEABLE: c_int = 210;

// Decoder errors.
pub const D_GIF_SUCCEEDED: c_int = 0;
pub const D_GIF_ERR_OPEN_FAILED: c_int = 101;
pub const D_GIF_ERR_READ_FAILED: c_int = 102;
pub const D_GIF_ERR_NOT_GIF_FILE: c_int = 103;
pub const D_GIF_ERR_NO_SCRN_DSCR: c_int = 104;
pub const D_GIF_ERR_NO_IMAG_DSCR: c_int = 105;
pub const D_GIF_ERR_NO_COLOR_MAP: c_int = 106;
pub const D_GIF_ERR_WRONG_RECORD: c_int = 107;
pub const D_GIF_ERR_DATA_TOO_BIG: c_int = 108;
pub const D_GIF_ERR_NOT_ENOUGH_MEM: c_int = 109;
pub const D_GIF_ERR_CLOSE_FAILED: c_int = 110;
pub const D_GIF_ERR_NOT_READABLE: c_int = 111;
pub const D_GIF_ERR_IMAGE_DEFECT: c_int = 112;
pub const D_GIF_ERR_EOF_TOO_SOON: c_int = 113;

// Layout guards: these were measured from the C header with gcc on x86_64.
// If any of them fail, the Rust structs no longer match gif_lib.h.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<GifFileType>() == 120);
    assert!(size_of::<SavedImage>() == 56);
    assert!(size_of::<GifImageDesc>() == 32);
    assert!(size_of::<ColorMapObject>() == 24);
    assert!(size_of::<ExtensionBlock>() == 24);
    assert!(size_of::<GraphicsControlBlock>() == 16);
    assert!(offset_of!(GifFileType, Image) == 40);
    assert!(offset_of!(GifFileType, SavedImages) == 72);
    assert!(offset_of!(GifFileType, Error) == 96);
    assert!(offset_of!(GifFileType, Private) == 112);
};
