use std::os::windows::ffi::OsStrExt;
use std::path::Path;

type Filter = unsafe extern "C" fn(*mut u8, *mut u8, i64, i32);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> *const ();
    fn FreeLibrary(module: isize) -> i32;
}

pub(super) struct StrideCodec {
    handle: isize,
    decode: Filter,
    encode: Filter,
}

impl StrideCodec {
    pub fn open(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // ゲーム本体は起動せず、同じ圧縮フィルターをファイル変換に使う。
        let handle = unsafe { LoadLibraryExW(name.as_ptr(), 0, 0x100 | 0x1000) };
        if handle == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let decode = unsafe { GetProcAddress(handle, c"unfilter_strided_data".as_ptr().cast()) };
        let encode = unsafe { GetProcAddress(handle, c"filter_strided_data".as_ptr().cast()) };
        if decode.is_null() || encode.is_null() {
            unsafe { FreeLibrary(handle) };
            return Err("ストライド変換関数を取得できません".into());
        }
        Ok(Self {
            handle,
            decode: unsafe { std::mem::transmute::<*const (), Filter>(decode) },
            encode: unsafe { std::mem::transmute::<*const (), Filter>(encode) },
        })
    }

    pub fn transform(&self, input: &[u8], stride: u32, decode: bool) -> Result<Vec<u8>, String> {
        // ゲームは端数を持つバッファもフィルターに渡す。
        if stride == 0 || stride > i32::MAX as u32 {
            return Err("ストライドとバッファの長さが一致しません".into());
        }
        let mut output = vec![0; input.len()];
        // ゲームの unfilter は入力も作業領域として変更する。
        let mut scratch = input.to_vec();
        let function = if decode { self.decode } else { self.encode };
        unsafe {
            function(
                scratch.as_mut_ptr(),
                output.as_mut_ptr(),
                input.len() as i64,
                stride as i32,
            )
        };
        Ok(output)
    }
}

impl Drop for StrideCodec {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.handle) };
    }
}
