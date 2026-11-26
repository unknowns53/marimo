#[cfg(windows)]
pub use imp::{IconSource, app_icons};

#[cfg(windows)]
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

// アプリの画面は data URL の image/png として埋め込むので、それ以外の形式は変換せずに捨てる。
#[cfg(windows)]
fn png_only(bytes: Vec<u8>) -> Option<Vec<u8>> {
    bytes.starts_with(PNG_SIGNATURE).then_some(bytes)
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapPixelFormat, SoftwareBitmap,
    };
    use windows::Management::Deployment::PackageManager;
    use windows::Storage::FileProperties::{ThumbnailMode, ThumbnailOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{
        DataReader, DataWriter, IRandomAccessStream, InMemoryRandomAccessStream,
    };
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
    };
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
    };
    use windows::core::{Error, HSTRING, Interface, Result};

    /// アイコンを読む先。
    pub enum IconSource {
        /// MSIX のパッケージファミリー名。スタートメニューの一覧に出る最初のアプリのアイコンを読む。
        Package(&'static str),
        /// MSIX ではなく入るアプリの実行ファイル。エクスプローラーが出すのと同じアイコンを読む。
        Exe(PathBuf),
    }

    /// 読む先ごとにアイコンを PNG のバイト列で返す。入っていないアプリや、読めなかったアイコンは None になる。
    ///
    /// 呼び出し元のスレッドは COM の STA のことがあり、そこで非同期の操作の完了を待つと
    /// 止まりうるので、MTA に入れた専用のスレッドで読む。
    pub fn app_icons(sources: Vec<IconSource>, px: u32) -> Vec<Option<Vec<u8>>> {
        let count = sources.len();
        std::thread::spawn(move || {
            let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
            let icons = sources
                .iter()
                .map(|source| {
                    let read = match source {
                        IconSource::Package(family) => package_icon(family, px),
                        IconSource::Exe(path) => exe_icon(path, px),
                    };
                    read.ok().flatten().and_then(super::png_only)
                })
                .collect();
            if initialized {
                unsafe { RoUninitialize() };
            }
            icons
        })
        .join()
        .unwrap_or_else(|_| vec![None; count])
    }

    // Microsoft Learn の FindPackagesByUserSecurityIdPackageFamilyName の説明のとおり、SID に
    // 空の文字列を渡すと今の利用者のパッケージを探し、それには管理者の権限が要らない。
    // AppDisplayInfo.GetLogo は、頼む大きさによらずタイル用の Square150x150Logo を返す。タイル用のロゴは周りに
    // 大きな余白を取ってあり（Claude で幅の半分、Codex で 3 分の 1 にしか描かれていない）、パネルの 12px では
    // 点にしかならない。そこで、スタートメニューの一覧やタスクバーと同じアイコンを、アプリの ID からシェルで読む。
    fn package_icon(family: &str, px: u32) -> Result<Option<Vec<u8>>> {
        let manager = PackageManager::new()?;
        let packages = manager.FindPackagesByUserSecurityIdPackageFamilyName(
            &HSTRING::new(),
            &HSTRING::from(family),
        )?;
        let Some(package) = packages.into_iter().next() else {
            return Ok(None);
        };
        let Some(entry) = package.GetAppListEntries()?.into_iter().next() else {
            return Ok(None);
        };
        let path = HSTRING::from(format!("shell:AppsFolder\\{}", entry.AppUserModelId()?));
        let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&path, None)? };
        let side = px as i32;
        // SIIGBF_ICONONLY は、縮小した画像ではなく、その大きさのために描かれたアイコンの画像を選ばせる。
        let hbitmap = unsafe { factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_ICONONLY)? };
        let bitmap = software_bitmap(hbitmap);
        unsafe {
            let _ = DeleteObject(hbitmap.into());
        }
        encode_png(&bitmap?).map(Some)
    }

    // シェルが返すビットマップは、アルファを掛けた 32 ビットの BGRA である。上下の向きによらず上の行から並べて取り出し、
    // PNG に書く前にアルファを掛けない値へ戻す。
    fn software_bitmap(hbitmap: HBITMAP) -> Result<SoftwareBitmap> {
        let mut info = BITMAP::default();
        let read = unsafe {
            GetObjectW(
                hbitmap.into(),
                size_of::<BITMAP>() as i32,
                Some((&raw mut info).cast()),
            )
        };
        if read == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
            return Err(Error::from_thread());
        }
        let (width, height) = (info.bmWidth, info.bmHeight);
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // 高さを負にすると、上の行から並べて返す。
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let lines = unsafe {
            let dc = GetDC(None);
            let lines = GetDIBits(
                dc,
                hbitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut header,
                DIB_RGB_COLORS,
            );
            ReleaseDC(None, dc);
            lines
        };
        if lines != height {
            return Err(Error::from_thread());
        }
        let writer = DataWriter::new()?;
        writer.WriteBytes(&pixels)?;
        let premultiplied = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &writer.DetachBuffer()?,
            BitmapPixelFormat::Bgra8,
            width,
            height,
            BitmapAlphaMode::Premultiplied,
        )?;
        SoftwareBitmap::ConvertWithAlpha(
            &premultiplied,
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Straight,
        )
    }

    // シェルのサムネイルは PNG とは限らないので、デコードして PNG に書き直す。
    fn exe_icon(path: &std::path::Path, px: u32) -> Result<Option<Vec<u8>>> {
        if !path.is_file() {
            return Ok(None);
        }
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
        let thumbnail = file
            .GetThumbnailAsync(
                ThumbnailMode::SingleItem,
                px,
                ThumbnailOptions::ResizeThumbnail,
            )?
            .join()?;
        let decoder =
            BitmapDecoder::CreateAsync(&thumbnail.cast::<IRandomAccessStream>()?)?.join()?;
        let bitmap = decoder
            .GetSoftwareBitmapConvertedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Straight)?
            .join()?;
        encode_png(&bitmap).map(Some)
    }

    fn encode_png(bitmap: &SoftwareBitmap) -> Result<Vec<u8>> {
        let png: IRandomAccessStream = InMemoryRandomAccessStream::new()?.cast()?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::PngEncoderId()?, &png)?.join()?;
        encoder.SetSoftwareBitmap(bitmap)?;
        encoder.FlushAsync()?.join()?;
        Ok(read_all(&png)?.unwrap_or_default())
    }

    fn read_all(stream: &IRandomAccessStream) -> Result<Option<Vec<u8>>> {
        let Ok(size) = u32::try_from(stream.Size()?) else {
            return Ok(None);
        };
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        let loaded = reader.LoadAsync(size)?.join()?;
        let mut bytes = vec![0; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(Some(bytes))
    }
}
