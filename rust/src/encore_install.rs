//! Independent first-install engine choices and per-game component selection.
use crate::{presets::Values, scanner::Evidence};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frames {
    None,
    SmoothMotion,
    Dlss,
    Xefg,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconstruction {
    None,
    Dlss,
    Xess,
    Nis,
    Fsr1,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub exe: String,
    pub name: String,
    pub needs_frames: bool,
    pub needs_reconstruction: bool,
    pub dlss_fg_available: bool,
    pub nis_available: bool,
    pub fsr1_available: bool,
    pub frames: Option<Frames>,
    pub reconstruction: Option<Reconstruction>,
    pub screen: bool,
    pub installed: bool,
    pub native_streamline: bool,
    pub native_fg: bool,
    pub components: crate::encore_components::Components,
}
impl Choice {
    pub fn new(exe: String, name: String, e: &Evidence, dlss_fg_available: bool) -> Self {
        Self {
            exe,
            name,
            needs_frames: !e.native_frame_generation(),
            needs_reconstruction: !e.neural_input() && !e.super_resolution_plugin,
            dlss_fg_available,
            nis_available: false,
            fsr1_available: false,
            frames: None,
            reconstruction: None,
            screen: false,
            installed: false,
            native_streamline: e.frame_integration_paths.iter().any(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("sl."))
            }),
            components: crate::encore_components::Components {
                fg: e.frame_generation,
                added_fg: dlss_fg_available,
                ..Default::default()
            },
            native_fg: e.frame_generation,
        }
    }
    pub fn needed(&self) -> bool {
        self.needs_frames || self.needs_reconstruction
    }
    pub fn values(&self) -> Option<Values> {
        if !self.installed && !self.components.patch {
            return None;
        }
        let mut v = Values::new();
        if self.needs_frames {
            let f = self.frames?;
            if f == Frames::Dlss && !self.dlss_fg_available {
                return None;
            }
            if f == Frames::Dlss && !self.components.added_fg
                || f == Frames::Xefg && !self.components.intel
            {
                return None;
            }
            v.insert(
                "frameGenerationWithoutGameFg".into(),
                if matches!(f, Frames::Dlss | Frames::Xefg) {
                    "1"
                } else {
                    "0"
                }
                .into(),
            );
            v.insert(
                "smoothMotionEnabled".into(),
                if f == Frames::SmoothMotion { "1" } else { "0" }.into(),
            );
            v.insert(
                "frameGenerationEngine".into(),
                if f == Frames::Xefg { "xefg" } else { "game" }.into(),
            );
        }
        if self.needs_reconstruction {
            let r = self.reconstruction?;
            if r == Reconstruction::Nis && !self.nis_available
                || r == Reconstruction::Fsr1 && !self.fsr1_available
            {
                return None;
            }
            if r == Reconstruction::Dlss && !self.components.sr
                || r == Reconstruction::Xess && !self.components.intel
            {
                return None;
            }
            v.insert(
                "dlssWithoutGameDlss".into(),
                if r == Reconstruction::None { "0" } else { "1" }.into(),
            );
            v.insert(
                "dlssWithoutGameDlssEngine".into(),
                if r == Reconstruction::Fsr1 {
                    "fsr1"
                } else if r == Reconstruction::Nis {
                    "nis"
                } else if r == Reconstruction::Xess {
                    "xess"
                } else {
                    "dlss"
                }
                .into(),
            );
            v.insert(
                "dlssWithoutGameDlssOutput".into(),
                if self.screen && r != Reconstruction::None {
                    "screen"
                } else {
                    "game"
                }
                .into(),
            );
        }
        Some(v)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selections_are_independent_explicit_and_default_to_game_resolution() {
        let mut c = Choice::new("game.exe".into(), "Game".into(), &Evidence::default(), true);
        assert!(c.needed());
        assert!(c.values().is_none());
        c.frames = Some(Frames::None);
        assert!(c.values().is_none());
        c.reconstruction = Some(Reconstruction::Dlss);
        let v = c.values().unwrap();
        assert_eq!(v["dlssWithoutGameDlssOutput"], "game");
        assert_eq!(v["frameGenerationWithoutGameFg"], "0");
        c.frames = Some(Frames::Xefg);
        assert_eq!(c.values().unwrap()["frameGenerationEngine"], "xefg");
        c.frames = Some(Frames::SmoothMotion);
        let v = c.values().unwrap();
        assert_eq!(v["smoothMotionEnabled"], "1");
        assert_eq!(v["frameGenerationWithoutGameFg"], "0");
        c.frames = Some(Frames::Dlss);
        c.dlss_fg_available = false;
        assert!(c.values().is_none());
    }
    #[test]
    fn native_sr_is_not_native_fg_and_partial_inventory_is_not_absence() {
        let e = Evidence {
            super_resolution: true,
            ..Default::default()
        };
        let c = Choice::new("g.exe".into(), "G".into(), &e, false);
        assert!(c.needs_frames);
        assert!(!c.needs_reconstruction);
        let e = Evidence {
            frame_integration_paths: vec!["sl.dlss.dll".into()],
            ..Default::default()
        };
        assert!(!e.native_frame_generation());
        let e = Evidence {
            frame_integration_paths: vec!["libxess_fg.dll".into()],
            ..Default::default()
        };
        assert!(e.native_frame_generation());
    }
}
