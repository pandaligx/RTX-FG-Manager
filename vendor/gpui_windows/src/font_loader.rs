//! Owned in-memory fonts using the original DirectWrite loader interfaces.
//! Unlike IDWriteInMemoryFontFileLoader, these work before Windows 10 1703.

use parking_lot::RwLock;
use std::{ffi::c_void, sync::Arc};
use windows::{
    Win32::{Foundation::E_INVALIDARG, Graphics::DirectWrite::*},
    core::*,
};

type FontData = Arc<RwLock<Vec<Arc<[u8]>>>>;

pub(crate) struct FontLoader {
    pub(crate) interface: IDWriteFontFileLoader,
    data: FontData,
}

impl FontLoader {
    pub(crate) fn new() -> Self {
        let data = FontData::default();
        Self {
            interface: MemoryFontLoader { data: data.clone() }.into(),
            data,
        }
    }

    pub(crate) fn add_file(
        &self,
        factory: &IDWriteFactory3,
        bytes: &[u8],
    ) -> Result<IDWriteFontFile> {
        let key = {
            let mut data = self.data.write();
            let key = u32::try_from(data.len()).map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            data.push(Arc::from(bytes));
            key
        };
        // DirectWrite copies the key; the loader and streams retain the font bytes.
        unsafe {
            factory.CreateCustomFontFileReference(
                (&key as *const u32).cast(),
                size_of::<u32>() as u32,
                &self.interface,
            )
        }
    }
}

#[implement(IDWriteFontFileLoader)]
struct MemoryFontLoader {
    data: FontData,
}

impl IDWriteFontFileLoader_Impl for MemoryFontLoader_Impl {
    fn CreateStreamFromKey(
        &self,
        key: *const c_void,
        key_size: u32,
    ) -> Result<IDWriteFontFileStream> {
        if key.is_null() || key_size != size_of::<u32>() as u32 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        // The COM caller supplies key_size readable bytes; alignment is unspecified.
        let index = unsafe { key.cast::<u32>().read_unaligned() } as usize;
        let data = self
            .data
            .read()
            .get(index)
            .cloned()
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        Ok(MemoryFontStream { data }.into())
    }
}

#[implement(IDWriteFontFileStream)]
struct MemoryFontStream {
    data: Arc<[u8]>,
}

fn fragment_range(len: usize, offset: u64, size: u64) -> Result<std::ops::Range<usize>> {
    let end = offset
        .checked_add(size)
        .filter(|&end| end <= len as u64)
        .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
    Ok(offset as usize..end as usize)
}

impl IDWriteFontFileStream_Impl for MemoryFontStream_Impl {
    fn ReadFileFragment(
        &self,
        start: *mut *mut c_void,
        offset: u64,
        size: u64,
        context: *mut *mut c_void,
    ) -> Result<()> {
        if start.is_null() || context.is_null() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let range = fragment_range(self.data.len(), offset, size)?;
        // Immutable bytes remain valid for the entire stream lifetime, including
        // concurrent reads. DirectWrite keeps the stream alive until fragments release.
        unsafe {
            *start = self.data.as_ptr().add(range.start).cast_mut().cast();
            *context = std::ptr::null_mut();
        }
        Ok(())
    }

    fn ReleaseFileFragment(&self, _context: *mut c_void) {}
    fn GetFileSize(&self) -> Result<u64> {
        Ok(self.data.len() as u64)
    }
    fn GetLastWriteTime(&self) -> Result<u64> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_reject_overflow_and_out_of_bounds() {
        assert_eq!(fragment_range(8, 2, 6).unwrap(), 2..8);
        assert_eq!(fragment_range(8, 8, 0).unwrap(), 8..8);
        assert!(fragment_range(8, 9, 0).is_err());
        assert!(fragment_range(8, 7, 2).is_err());
        assert!(fragment_range(8, u64::MAX, 2).is_err());
    }

    #[test]
    fn stream_keeps_owned_font_bytes_alive_after_loader_drops() {
        let loader = FontLoader::new();
        loader.data.write().push(Arc::from([10u8, 20, 30, 40]));
        let key = 0u32;
        let stream = unsafe {
            loader
                .interface
                .CreateStreamFromKey((&key as *const u32).cast(), 4)
                .unwrap()
        };
        drop(loader);
        let mut start = std::ptr::null_mut();
        let mut context = std::ptr::null_mut();
        unsafe {
            stream
                .ReadFileFragment(&mut start, 1, 2, &mut context)
                .unwrap();
            assert_eq!(std::slice::from_raw_parts(start.cast::<u8>(), 2), &[20, 30]);
            stream.ReleaseFileFragment(context);
            assert!(
                stream
                    .ReadFileFragment(&mut start, 3, 2, &mut context)
                    .is_err()
            );
        }
    }

    #[test]
    fn directwrite_loads_owned_font_with_pre_1703_interfaces() {
        let font_path =
            std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("Fonts/segoeui.ttf");
        let bytes = std::fs::read(font_path).unwrap();
        let loader = FontLoader::new();
        unsafe {
            let factory: IDWriteFactory3 =
                DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED).unwrap();
            factory.RegisterFontFileLoader(&loader.interface).unwrap();
            let file = loader.add_file(&factory, &bytes).unwrap();
            drop(bytes);
            let mut supported = BOOL(0);
            let mut file_type = DWRITE_FONT_FILE_TYPE_UNKNOWN;
            let mut face_count = 0;
            file.Analyze(&mut supported, &mut file_type, None, &mut face_count)
                .unwrap();
            assert!(supported.as_bool() && face_count > 0);
            let builder = factory.CreateFontSetBuilder().unwrap();
            for index in 0..face_count {
                let face = factory
                    .CreateFontFaceReference(&file, index, DWRITE_FONT_SIMULATIONS_NONE)
                    .unwrap();
                builder.AddFontFaceReference2(&face).unwrap();
            }
            let set = builder.CreateFontSet().unwrap();
            assert_eq!(set.GetFontCount(), face_count);
            let face = set
                .GetFontFaceReference(0)
                .unwrap()
                .CreateFontFace()
                .unwrap();
            assert!(face.GetGlyphCount() > 0);
            drop((face, set, builder, file));
            factory.UnregisterFontFileLoader(&loader.interface).unwrap();
        }
    }
}
