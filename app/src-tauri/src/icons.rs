use base64::Engine;
use serde::Serialize;

// パネルでは 12〜16 CSS px で出すので、Retina でも粗くならない大きさで読む。
#[cfg(any(target_os = "macos", windows))]
const ICON_PX: u32 = 64;

/// 利用者が入れているアプリのアイコン。アイコンは同梱せず、読めなければ画面が文字の印に切り替える。
/// Hermes はデスクトップアプリを持たないので、いつも文字の印で出す。
#[derive(Debug, Clone, Default, Serialize)]
pub struct AppIcons {
    pub claude: Option<String>,
    pub codex: Option<String>,
    pub hermes: Option<String>,
}

pub fn load() -> AppIcons {
    let [claude, codex] = platform_pngs();
    AppIcons {
        claude: claude.map(data_url),
        codex: codex.map(data_url),
        hermes: None,
    }
}

fn data_url(png: Vec<u8>) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

#[cfg(target_os = "macos")]
fn platform_pngs() -> [Option<Vec<u8>>; 2] {
    // Codex のデスクトップアプリは ChatGPT.app として入り、bundle id は com.openai.codex である。
    ["com.anthropic.claudefordesktop", "com.openai.codex"].map(mac::app_icon_png)
}

#[cfg(windows)]
fn platform_pngs() -> [Option<Vec<u8>>; 2] {
    let mut logos = marimo_core::appicon::package_logos(
        &["Claude_pzs8sxrjxfjjc", "OpenAI.Codex_2p2nqsd0c76g0"],
        ICON_PX as f32,
    )
    .into_iter();
    [logos.next().flatten(), logos.next().flatten()]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_pngs() -> [Option<Vec<u8>>; 2] {
    [None, None]
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    use super::ICON_PX;

    /// bundle id のアプリが入っていれば、NSWorkspace が返すアイコンを PNG にして返す。
    pub fn app_icon_png(bundle_id: &str) -> Option<Vec<u8>> {
        let workspace = NSWorkspace::sharedWorkspace();
        let url =
            workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
        let path = url.path()?;
        let icon = workspace.iconForFile(&path);

        let px = ICON_PX as isize;
        // SAFETY: planes に null を渡すと、NSBitmapImageRep が自分で画素の領域を確保する。
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                px,
                px,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                0,
                0,
            )
        }?;
        let side = f64::from(ICON_PX);
        rep.setSize(NSSize::new(side, side));
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        let previous = NSGraphicsContext::currentContext();
        NSGraphicsContext::setCurrentContext(Some(&context));
        // 描く先の大きさに合う解像度の表現を NSImage が選ぶので、縮小は AppKit に任せる。
        icon.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)),
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        context.flushGraphics();
        NSGraphicsContext::setCurrentContext(previous.as_deref());
        // SAFETY: 空の辞書は、どの型の値も持たないので型の取り違えが起こらない。
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(png.to_vec())
    }
}
