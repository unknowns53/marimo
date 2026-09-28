#[cfg(windows)]
pub use imp::package_logos;

#[cfg(any(windows, test))]
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

// アプリの画面は data URL の image/png として埋め込むので、それ以外の形式は変換せずに捨てる。
#[cfg(any(windows, test))]
fn png_only(bytes: Vec<u8>) -> Option<Vec<u8>> {
    bytes.starts_with(PNG_SIGNATURE).then_some(bytes)
}

#[cfg(windows)]
mod imp {
    use windows::Foundation::Size;
    use windows::Management::Deployment::PackageManager;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    use windows::core::{HSTRING, Result};

    /// MSIX のパッケージファミリー名ごとに、最初のアプリのロゴを PNG のバイト列で返す。
    /// 入っていないパッケージや、読めなかったロゴは None になる。
    ///
    /// 呼び出し元のスレッドは COM の STA のことがあり、そこで非同期の操作の完了を待つと
    /// 止まりうるので、MTA に入れた専用のスレッドで読む。
    pub fn package_logos(families: &[&str], px: f32) -> Vec<Option<Vec<u8>>> {
        let families: Vec<String> = families.iter().map(|f| (*f).to_owned()).collect();
        let count = families.len();
        std::thread::spawn(move || {
            let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
            let logos = families
                .iter()
                .map(|family| logo(family, px).ok().flatten().and_then(super::png_only))
                .collect();
            if initialized {
                unsafe { RoUninitialize() };
            }
            logos
        })
        .join()
        .unwrap_or_else(|_| vec![None; count])
    }

    // Microsoft Learn の FindPackagesByUserSecurityIdPackageFamilyName の説明のとおり、SID に
    // 空の文字列を渡すと今の利用者のパッケージを探し、それには管理者の権限が要らない。
    fn logo(family: &str, px: f32) -> Result<Option<Vec<u8>>> {
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
        let reference = entry.DisplayInfo()?.GetLogo(Size {
            Width: px,
            Height: px,
        })?;
        let stream = reference.OpenReadAsync()?.join()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_png() {
        let png = [PNG_SIGNATURE, b"rest"].concat();
        assert_eq!(png_only(png.clone()), Some(png));
        for other in [b"GIF89a".to_vec(), Vec::new(), PNG_SIGNATURE[..4].to_vec()] {
            assert_eq!(png_only(other), None);
        }
    }
}
