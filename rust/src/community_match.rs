//! Deterministic local ranking with explanations, never a performance guarantee.
use crate::community::{Hardware, Profile};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Filter {
    Mine,
    SameGpu,
    Confirmed,
    Official,
    Measured,
    NeuralRendering,
    SmoothMotion,
    Fixed,
    Dynamic,
}
pub const FILTERS: [Filter; 9] = [
    Filter::Mine,
    Filter::SameGpu,
    Filter::Confirmed,
    Filter::Official,
    Filter::Measured,
    Filter::NeuralRendering,
    Filter::SmoothMotion,
    Filter::Fixed,
    Filter::Dynamic,
];
impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mine => "我的配置",
            Self::SameGpu => "同显卡",
            Self::Confirmed => "玩家确认",
            Self::Official => "官方",
            Self::Measured => "有测量",
            Self::NeuralRendering => "NR",
            Self::SmoothMotion => "Smooth Motion",
            Self::Fixed => "固定插帧",
            Self::Dynamic => "动态插帧",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub show_other_gpus: bool,
    pub filters: std::collections::BTreeSet<Filter>,
    pub visible: usize,
}
pub struct Matches<'a> {
    pub page: Vec<Matched<'a>>,
    pub total: usize,
    pub counts: BTreeMap<Filter, usize>,
}
pub struct Matched<'a> {
    pub profile: &'a Profile,
    pub reasons: Vec<&'static str>,
}
fn same(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a,b),(Some(a),Some(b)) if !a.trim().is_empty()&&a.trim().eq_ignore_ascii_case(b.trim()))
}
fn same_gpu(a: &Option<String>, b: &Option<String>) -> bool {
    fn normalize(s: &str) -> String {
        let lower = s.to_ascii_lowercase();
        let mut text = lower.trim();
        for prefix in ["nvidia ", "amd ", "intel ", "geforce ", "radeon "] {
            if let Some(rest) = text.strip_prefix(prefix) {
                text = rest;
            }
        }
        text.chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect()
    }
    matches!((a,b),(Some(a),Some(b)) if !a.trim().is_empty() && !b.trim().is_empty() && normalize(a)==normalize(b))
}
fn gpu_allowed(p: &Profile, h: &Hardware, show_other: bool) -> bool {
    show_other
        || h.gpu_model.as_ref().is_none_or(|s| s.trim().is_empty())
        || same_gpu(&p.content.hardware.gpu_model, &h.gpu_model)
}
fn passes(p: &Profile, h: &Hardware, identity: Option<&str>, filter: Filter) -> bool {
    let settings = &p.content.settings;
    let field = |k: &str| settings.get(k).map(String::as_str).unwrap_or("");
    let mode = field("tf_mode");
    let mfg = field("rtx_mode");
    match filter {
        Filter::Mine => identity == Some(p.author.as_str()),
        Filter::SameGpu => same_gpu(&p.content.hardware.gpu_model, &h.gpu_model),
        Filter::Confirmed => p.works > 0,
        Filter::Official => p.official,
        Filter::Measured => p.content.measurement.as_ref().is_some_and(|m| {
            m.base_fps.is_some() || m.output_fps.is_some() || m.one_percent_low.is_some()
        }),
        Filter::NeuralRendering => field("nrEnabled") == "1",
        Filter::SmoothMotion => field("smoothMotionEnabled") == "1",
        Filter::Dynamic => mode == "dynamic" || mfg == "dynamic",
        Filter::Fixed => {
            [mode, mfg]
                .iter()
                .any(|s| s.parse::<u32>().is_ok_and(|v| (2..=6).contains(&v)))
                || field("max_generated_frames")
                    .parse::<u32>()
                    .is_ok_and(|v| (1..=5).contains(&v))
        }
    }
}
fn driver_number(driver: &str, model: Option<&str>) -> Option<u64> {
    let mut parts = driver
        .trim()
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let model = model.unwrap_or("").to_ascii_lowercase();
    if model.contains("nvidia")
        || model.contains("geforce")
        || model.starts_with("rtx")
        || model.starts_with("gtx")
    {
        if parts.len() == 4 {
            return Some(u64::from(parts[2] % 10) * 10000 + u64::from(parts[3]));
        }
        if parts.len() == 2 && parts[1] < 100 {
            return Some(u64::from(parts[0]) * 100 + u64::from(parts[1]));
        }
    }
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts.resize(4, 0);
    Some(parts.into_iter().fold(0, |n, p| (n << 16) | u64::from(p)))
}
fn driver_distance(a: &Hardware, b: &Hardware) -> u64 {
    if same(&a.driver, &b.driver) {
        return 0;
    }
    match (
        a.driver
            .as_deref()
            .and_then(|v| driver_number(v, a.gpu_model.as_deref())),
        b.driver
            .as_deref()
            .and_then(|v| driver_number(v, b.gpu_model.as_deref())),
    ) {
        (Some(a), Some(b)) => a.abs_diff(b),
        _ => u64::MAX,
    }
}
fn rank<'a>(p: &'a Profile, h: &Hardware) -> (u8, u64, u32, u32, std::cmp::Reverse<i64>, &'a str) {
    let other = &p.content.hardware;
    let gpu = if same_gpu(&other.gpu_model, &h.gpu_model) {
        0
    } else if other.gpu_model.is_some() {
        1
    } else {
        2
    };
    (
        gpu,
        driver_distance(other, h),
        u32::MAX - p.works,
        u32::MAX - p.likes,
        std::cmp::Reverse(p.updated_at),
        p.id.as_str(),
    )
}
pub fn select<'a>(
    game: &str,
    profiles: &'a [Profile],
    hardware: &Hardware,
    scheme: &str,
    patch: &str,
    identity: Option<&str>,
    selection: &Selection,
) -> Matches<'a> {
    let eligible = profiles
        .iter()
        .filter(|p| crate::config_store::valid_game(game) && p.game == game)
        .filter(|p| gpu_allowed(p, hardware, selection.show_other_gpus))
        .collect::<Vec<_>>();
    let counts = FILTERS
        .into_iter()
        .map(|filter| {
            (
                filter,
                eligible
                    .iter()
                    .filter(|p| {
                        passes(p, hardware, identity, filter)
                            && selection
                                .filters
                                .iter()
                                .all(|f| *f == filter || passes(p, hardware, identity, *f))
                    })
                    .count(),
            )
        })
        .collect();
    let mut selected = eligible
        .into_iter()
        .filter(|p| {
            selection
                .filters
                .iter()
                .all(|f| passes(p, hardware, identity, *f))
        })
        .collect::<Vec<_>>();
    selected.sort_by(|a, b| rank(a, hardware).cmp(&rank(b, hardware)));
    let total = selected.len();
    let page = selected
        .into_iter()
        .take(selection.visible.max(12))
        .map(|p| {
            let other = &p.content.hardware;
            let mut reasons = Vec::new();
            if same_gpu(&other.gpu_model, &hardware.gpu_model) {
                reasons.push("同型号显卡");
            }
            if driver_distance(other, hardware) == 0 {
                reasons.push("相同驱动版本");
            } else if let (Some(a), Some(b)) = (&other.driver, &hardware.driver)
                && let (Some(a), Some(b)) = (
                    driver_number(a, other.gpu_model.as_deref()),
                    driver_number(b, hardware.gpu_model.as_deref()),
                )
                && a < b
            {
                reasons.push("玩家驱动较旧");
            }
            if p.scheme == scheme && p.content.patch_version == patch {
                reasons.push("相同补丁版本");
            }
            if other.gpu_model.is_none() || other.driver.is_none() {
                reasons.push("部分硬件信息缺失");
            }
            Matched {
                profile: p,
                reasons,
            }
        })
        .collect();
    Matches {
        page,
        total,
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::community::Content;
    fn profile(n: u32, gpu: &str) -> Profile {
        Profile {
            id: format!("00000000-0000-4000-8000-{n:012}"),
            author: "a".repeat(64),
            game: "steam:123".into(),
            scheme: crate::encore::SCHEME.into(),
            protocol: crate::encore::PROFILE.into(),
            revision: 1,
            official: false,
            updated_at: 1,
            likes: 0,
            works: 0,
            status: None,
            content: Content {
                title: "Test".into(),
                notes: String::new(),
                settings: BTreeMap::new(),
                hardware: Hardware {
                    gpu_model: Some(gpu.into()),
                    driver: Some("616.92".into()),
                },
                patch_version: "1.0.0-rc.3".into(),
                entry: Some("version.dll".into()),
                runtime_versions: BTreeMap::new(),
                dlss_mode: None,
                ray_tracing: None,
                measurement: None,
            },
        }
    }
    #[test]
    fn other_games_never_enter_recommendations_or_filter_counts() {
        let mut other = profile(1, "RTX 3060");
        other.game = "steam:456".into();
        other.works = 500;
        other.likes = 900;
        let mut alternative = profile(2, "RTX 3060");
        alternative.scheme = crate::upstream040::SCHEME.into();
        alternative.protocol = crate::upstream040::PROFILE.into();
        alternative.works = 2;
        let hardware = other.content.hardware.clone();
        let profiles = vec![other, profile(0, "RTX 3060"), alternative];
        let selection = Selection {
            show_other_gpus: true,
            ..Default::default()
        };
        let found = select(
            "steam:123",
            &profiles,
            &hardware,
            crate::encore::SCHEME,
            "1.0.0-rc.3",
            None,
            &selection,
        );
        assert_eq!(found.total, 2);
        assert_eq!(found.page[0].profile.id, profiles[2].id);
        assert_eq!(found.counts[&Filter::Confirmed], 1);
        assert!(!found.page[0].reasons.contains(&"相同补丁版本"));
        assert_eq!(
            select(
                "",
                &profiles,
                &hardware,
                crate::encore::SCHEME,
                "1.0.0-rc.3",
                None,
                &selection
            )
            .total,
            0
        );
    }
    #[test]
    fn driver_formats_and_proximity_match_without_other_hardware() {
        assert_eq!(
            driver_number("32.0.16.1692", Some("NVIDIA GeForce RTX 3060")),
            driver_number("616.92", Some("RTX 3060"))
        );
        assert_eq!(driver_number("not a version", None), None);
        let mut exact = profile(1, "RTX 3060");
        exact.content.hardware.driver = Some("32.0.16.1692".into());
        let mut close = profile(2, "RTX 3060");
        close.content.hardware.driver = Some("616.91".into());
        let mut old = profile(3, "RTX 3060");
        old.content.hardware.driver = Some("580.88".into());
        old.works = 100;
        let mut unknown = profile(4, "RTX 3060");
        unknown.content.hardware.driver = None;
        let hardware = profile(0, "RTX 3060").content.hardware;
        let profiles = vec![unknown, old, close, exact];
        let result = select(
            "steam:123",
            &profiles,
            &hardware,
            crate::encore::SCHEME,
            "1.0.0-rc.3",
            None,
            &Selection::default(),
        );
        assert_eq!(
            result
                .page
                .iter()
                .map(|p| &p.profile.id)
                .collect::<Vec<_>>(),
            [3, 2, 1, 0].map(|i| &profiles[i].id)
        );
        assert!(result.page[0].reasons.contains(&"相同驱动版本"));
        assert!(result.page[1].reasons.contains(&"玩家驱动较旧"));
    }
    #[test]
    fn useful_confirmations_then_likes_break_hardware_ties() {
        let mut liked = profile(1, "RTX 3060");
        liked.likes = 20;
        let mut confirmed = profile(2, "RTX 3060");
        confirmed.works = 1;
        let mut popular_other_gpu = profile(3, "RTX 4090");
        popular_other_gpu.works = 100;
        let hardware = liked.content.hardware.clone();
        let profiles = vec![profile(0, "RTX 3060"), liked, confirmed, popular_other_gpu];
        let selection = Selection {
            show_other_gpus: true,
            ..Default::default()
        };
        let matches = select(
            "steam:123",
            &profiles,
            &hardware,
            crate::encore::SCHEME,
            "1.0.0-rc.3",
            None,
            &selection,
        );
        assert_eq!(
            matches
                .page
                .iter()
                .map(|p| p.profile.id.clone())
                .collect::<Vec<_>>(),
            [2, 1, 0, 3].map(|i| profiles[i].id.clone())
        );
    }
    #[test]
    fn fixed_filter_counts_dlssg_generated_frames_without_treating_default_as_fixed() {
        let mut p = profile(1, "RTX 3060");
        p.content
            .settings
            .insert("max_generated_frames".into(), "1".into());
        let hardware = p.content.hardware.clone();
        assert!(passes(&p, &hardware, None, Filter::Fixed));
        assert!(!passes(&p, &hardware, None, Filter::Dynamic));
        p.content
            .settings
            .insert("max_generated_frames".into(), "0".into());
        assert!(!passes(&p, &hardware, None, Filter::Fixed));
        p.content.settings.clear();
        p.content.settings.insert("rtx_mode".into(), "1".into());
        assert!(!passes(&p, &hardware, None, Filter::Fixed));
        p.content
            .settings
            .insert("rtx_mode".into(), "dynamic".into());
        assert!(passes(&p, &hardware, None, Filter::Dynamic));
    }
    #[test]
    fn model_normalization_keeps_ti_and_laptop_distinct() {
        assert!(same_gpu(
            &Some("NVIDIA GeForce RTX 4060".into()),
            &Some("RTX4060".into())
        ));
        assert!(!same_gpu(
            &Some("RTX 4060 Ti".into()),
            &Some("RTX4060".into())
        ));
        assert!(!same_gpu(
            &Some("RTX 4060 Laptop GPU".into()),
            &Some("RTX4060".into())
        ));
    }
    #[test]
    fn ranking_pagination_filters_and_missing_data_are_local_and_deterministic() {
        let mut profiles = (0..15).map(|n| profile(n, "RTX 3060")).collect::<Vec<_>>();
        let hardware = profiles[0].content.hardware.clone();
        profiles[2].works = 4;
        profiles[1].content.hardware.driver = None;
        profiles.push(profile(20, "RTX 4060"));
        let selection = Selection::default();
        let result = select(
            "steam:123",
            &profiles,
            &hardware,
            crate::encore::SCHEME,
            "1.0.0-rc.3",
            None,
            &selection,
        );
        assert_eq!(result.total, 15);
        assert_eq!(result.page.len(), 12);
        assert_eq!(result.page[0].profile.id, profiles[2].id);
        assert_eq!(result.counts[&Filter::Confirmed], 1);
        assert_eq!(result.counts[&Filter::Mine], 0);
        let mut selection = Selection {
            show_other_gpus: true,
            visible: 24,
            ..Default::default()
        };
        assert_eq!(
            select(
                "steam:123",
                &profiles,
                &hardware,
                crate::encore::SCHEME,
                "1.0.0-rc.3",
                None,
                &selection
            )
            .total,
            16
        );
        selection.filters.insert(Filter::Confirmed);
        let result = select(
            "steam:123",
            &profiles,
            &hardware,
            crate::encore::SCHEME,
            "1.0.0-rc.3",
            None,
            &selection,
        );
        assert_eq!(result.total, 1);
        assert_eq!(result.counts[&Filter::SameGpu], 1);
    }
}
