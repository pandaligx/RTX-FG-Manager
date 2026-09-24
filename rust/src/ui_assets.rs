//! Official component icons plus the requested Bilibili brand exception, all offline.
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == "app/manager.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/manager.png"))));
        }
        if path == "app/refresh.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/refresh.svg"))));
        }
        if path == "app/bilibili.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/bilibili.svg"
            ))));
        }
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut files = gpui_component_assets::Assets.list(path)?;
        for name in ["app/manager.png", "app/refresh.svg", "app/bilibili.svg"] {
            if name.starts_with(path) {
                files.push(name.into());
            }
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_icons_and_windows_logo_are_available_offline() {
        let icons = Assets.list("icons/").unwrap();
        assert!(icons.len() > 80);
        for icon in icons {
            let bytes = Assets.load(&icon).unwrap().unwrap();
            let svg = std::str::from_utf8(&bytes).unwrap();
            assert!(svg.contains("<svg"), "{icon}");
            assert!(!svg.contains("Font Awesome"), "{icon}");
        }
        for path in [
            "icons/cpu.svg",
            "icons/github.svg",
            "icons/layout-dashboard.svg",
            "icons/triangle-alert.svg",
            "icons/check.svg",
            "icons/close.svg",
        ] {
            assert!(Assets.load(path).unwrap().is_some(), "{path}");
        }
        let logo = Assets.load("app/manager.png").unwrap().unwrap();
        let image = image::load_from_memory(&logo).unwrap();
        assert_eq!((image.width(), image.height()), (256, 256));
        let ico = image::load_from_memory_with_format(
            include_bytes!("../assets/manager.ico"),
            image::ImageFormat::Ico,
        )
        .unwrap();
        assert_eq!((ico.width(), ico.height()), (256, 256));
        assert!(Assets.load("app/nav-games.svg").is_err());
    }
}
