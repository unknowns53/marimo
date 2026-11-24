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

    use windows::Foundation::Size;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapPixelFormat,
    };
    use windows::Management::Deployment::PackageManager;
    use windows::Storage::FileProperties::{ThumbnailMode, ThumbnailOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, IRandomAccessStream, InMemoryRandomAccessStream};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    use windows::core::{HSTRING, Interface, Result};

    /// アイコンを読む先。
    pub enum IconSource {
        /// MSIX のパッケージファミリー名。最初のアプリのロゴを読む。
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
                        IconSource::Package(family) => package_logo(family, px),
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
    fn package_logo(family: &str, px: u32) -> Result<Option<Vec<u8>>> {
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
        let side = px as f32;
        let reference = entry.DisplayInfo()?.GetLogo(Size {
            Width: side,
            Height: side,
        })?;
        let stream = reference.OpenReadAsync()?.join()?;
        read_all(&stream.cast()?)
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
        let png: IRandomAccessStream = InMemoryRandomAccessStream::new()?.cast()?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::PngEncoderId()?, &png)?.join()?;
        encoder.SetSoftwareBitmap(&bitmap)?;
        encoder.FlushAsync()?.join()?;
        read_all(&png)
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
