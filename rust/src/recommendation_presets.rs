//! Public parameter snapshots. Never accept paths, commands, diagnostic settings
//! or key bindings from a recommendation. Defaults travel with the snapshot so
//! a later patch cannot silently change the meaning of the comparison.
use crate::{
    cloud, core, encore,
    presets::{self, Values},
    rtxmfg,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub profile: String,
    pub values: Values,
    pub defaults: Values,
}

pub const PROFILES: &[(&str, &str)] = &[
    ("dlssg-transfusion-1.4.5.3", encore::PROFILE),
    ("rtx40mfg-1.3.3-hf2", rtxmfg::PROFILE),
    ("upstream-0.3.5-310-9", crate::upstream040::PROFILE),
    ("rtxfg-0.3.5-dx12-vulkan", "upstream035"),
    ("pipotoufik-mfg-vulkan-sm86-7", presets::MFG_VULKAN),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    InstallAndApply,
    InstallOnly,
    ParametersOnly,
}
impl Action {
    pub fn installs(self) -> bool {
        self != Self::ParametersOnly
    }
    pub fn parameters(self) -> bool {
        self != Self::InstallOnly
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::InstallAndApply => "安装并应用",
            Self::InstallOnly => "仅安装方案",
            Self::ParametersOnly => "仅应用参数",
        }
    }
}

/// Preflight every destination before touching the first one. Per-target writes
/// use the existing ownership records, original backups and upgrade journals.
pub fn execute(
    game: &crate::scanner::Game,
    catalog: &cloud::Catalog,
    recipe: &crate::recommendations::Recipe,
    action: Action,
    series: usize,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(cloud::CloudProgress),
) -> Result<()> {
    use std::sync::atomic::Ordering;
    let version = recipe.available_version(catalog)?;
    let settings = if action.parameters() {
        let settings = recipe
            .presets
            .as_ref()
            .context("此分享未附带参数，请选择仅安装方案")?;
        settings.validate(&recipe.scheme, &version)?;
        Some(settings)
    } else {
        None
    };
    let targets = game.deployment_targets();
    ensure!(!targets.is_empty(), "未找到游戏的部署目录");
    if action == Action::InstallAndApply
        && let Some(runtimes) = &recipe.runtimes
    {
        runtimes.preflight(&targets, recipe)?;
    }
    let policy = catalog
        .scheme_policies
        .get(&recipe.scheme)
        .context("未知方案")?;
    for target in &targets {
        core::assert_target_stopped(target)?;
        if let Some(settings) = settings {
            let context = presets::Context::new(&recipe.scheme, policy, &target.game_exe);
            let mut normalized = settings.values.clone();
            context.normalize(&mut normalized);
            ensure!(
                normalized == settings.values,
                "此游戏不支持分享中的部分参数，请手动调整后应用"
            );
        }
        if action.installs() {
            core::preflight_install_at(target, &recipe.entries)?;
        } else {
            let record = core::record(&target.directory)?.context("请先安装当前方案再应用参数")?;
            target.validate_record(&record)?;
            ensure!(
                record.scheme_id.as_deref() == Some(&recipe.scheme)
                    && core::backend_matches_profile(&record.backend, &policy.parameter_profile),
                "已安装方案不同，请使用安装并应用"
            );
            let installed = record
                .upstream_version
                .as_deref()
                .or(record.payload_version.as_deref())
                .context("无法确认已安装补丁版本")?;
            settings.unwrap().validate(&recipe.scheme, installed)?;
            ensure!(
                core::status_at(target).starts_with("已部署"),
                "补丁文件已变化，请刷新后检查"
            );
        }
    }
    if let Some(settings) = settings {
        ensure!(
            !(settings.requires_xemfg() && recipe.entries.iter().any(|p| p.ends_with(".asi"))),
            "此分享启用了独立 XeSS，但选择了插件入口；请先修正分享方案"
        );
        if !action.installs() {
            for target in &targets {
                ensure!(
                    !settings.requires_nr() || crate::neural_runtime::inspect(target).present,
                    "缺少 DLSSNR 运行库，请使用安装并应用或先安装运行库"
                );
                ensure!(
                    !settings.requires_xemfg()
                        || crate::runtime_files::inspect(target, "xemfg").managed,
                    "缺少 XeSS 运行库，请使用安装并应用或先安装运行库"
                );
            }
        }
    }
    if action.installs() {
        let prepared = cloud::prepare_with_progress(
            catalog,
            &recipe.scheme,
            series,
            &recipe.entries,
            cancel,
            &mut progress,
        )?;
        if action == Action::InstallAndApply
            && let Some(runtimes) = &recipe.runtimes
        {
            runtimes.verify_prepared(&prepared.files)?;
        }
        for target in &targets {
            ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
            let context = presets::Context::new(&recipe.scheme, &prepared.policy, &target.game_exe);
            // No shared overrides in this phase. Existing user configuration is
            // retained until dependencies are ready and all values validated.
            core::deploy_prepared_context_at_with_overrides(
                target,
                &prepared.backend,
                &recipe.entries,
                None,
                prepared.files.clone(),
                Some(&prepared.version),
                Some(&context),
                None,
                prepared.upstream_version.as_deref(),
            )?;
        }
    }
    if let Some(settings) = settings {
        if action == Action::InstallAndApply {
            if let Some(runtimes) = &recipe.runtimes {
                runtimes
                    .install(&targets, catalog.prefer_github, cancel, &mut progress)
                    .context("方案已安装，配套运行库未完成；参数尚未应用，可重试")?;
            }
            let shared_nr = recipe
                .runtimes
                .as_ref()
                .is_some_and(|r| r.components.iter().any(|c| c.kind == "nr"));
            let shared_xess = recipe
                .runtimes
                .as_ref()
                .is_some_and(|r| r.components.iter().any(|c| c.source == "managed"));
            if !shared_nr && (settings.requires_nr() || recipe.neural_runtime == "310.9.16") {
                crate::neural_runtime::install_many(
                    &targets,
                    "310.9.16",
                    catalog.prefer_github,
                    cancel,
                    &mut |transfer| {
                        progress(cloud::CloudProgress {
                            transfer,
                            proxy: "nvngx_dlssnr.dll".into(),
                            package_index: 1,
                            package_count: 1,
                        })
                    },
                )
                .context("方案已安装，DLSSNR 未完成；参数尚未应用，可重试")?;
            }
            if settings.requires_xemfg() && !shared_xess {
                crate::runtime_files::install_many(
                    &targets,
                    "xemfg",
                    catalog.prefer_github,
                    cancel,
                    &mut |transfer| {
                        progress(cloud::CloudProgress {
                            transfer,
                            proxy: "xemfg".into(),
                            package_index: 1,
                            package_count: 1,
                        })
                    },
                )
                .context("方案已安装，XeSS 未完成；参数尚未应用，可重试")?;
            }
            if let Some(runtimes) = &recipe.runtimes {
                runtimes.verify_installed(&targets)?;
            }
        }
        for target in &targets {
            ensure!(
                !cancel.load(Ordering::Relaxed),
                "操作已取消，请刷新查看已完成项目"
            );
            let context = presets::Context::new(&recipe.scheme, policy, &target.game_exe);
            core::apply_parameters_at(target, &context, &settings.values).with_context(|| {
                format!("参数未完整应用，可重试：{}", target.directory.display())
            })?;
        }
    }
    Ok(())
}

pub fn shareable(profile: &str, key: &str) -> bool {
    if profile == encore::PROFILE {
        return encore::field(key).is_some_and(|f| {
            matches!(f.group.as_str(), "frames" | "image") && f.kind != encore::FieldKind::Hotkey
        });
    }
    PROFILES.iter().any(|(_, p)| *p == profile)
        && presets::parameters(profile).iter().any(|p| p.key == key)
        && !matches!(key, "rtx_opti_ui" | "overlay" | "debug" | "logging")
}

pub fn supported(profile: &str, key: &str, version: &str) -> bool {
    if !shareable(profile, key) {
        return false;
    }
    if profile == encore::PROFILE {
        return encore::field(key).is_some_and(|f| encore::field_available(f, Some(version)));
    }
    if profile == rtxmfg::PROFILE {
        return rtxmfg::validate_version(Some(version), &Values::from([(key.into(), "0".into())]))
            .is_ok();
    }
    true
}

impl Snapshot {
    pub fn validate(&self, scheme: &str, version: &str) -> Result<()> {
        ensure!(
            self.schema == 1 && PROFILES.contains(&(scheme, self.profile.as_str())),
            "分享参数属于其他方案或不支持的配置协议"
        );
        ensure!(
            !self.values.is_empty()
                && self.values.len() <= 192
                && self.values.keys().eq(self.defaults.keys()),
            "分享参数或默认值不完整"
        );
        for (key, value) in self.values.iter().chain(&self.defaults) {
            ensure!(
                supported(&self.profile, key, version),
                "此版本不支持分享参数：{key}"
            );
            ensure!(
                value.len() <= 64
                    && !value.is_empty()
                    && value.trim() == value
                    && !value
                        .chars()
                        .any(|c| c.is_control() || "/\\:<>{}".contains(c)),
                "分享参数包含无效内容：{key}"
            );
        }
        presets::validate(&self.profile, &self.values)?;
        presets::validate(&self.profile, &self.defaults)?;
        Ok(())
    }

    pub fn changes(&self) -> Vec<(&str, &str, &str)> {
        self.values
            .iter()
            .filter_map(|(key, value)| {
                let default = self.defaults.get(key)?;
                let same = value == default
                    || value
                        .parse::<f64>()
                        .ok()
                        .zip(default.parse::<f64>().ok())
                        .is_some_and(|(a, b)| a == b);
                (!same).then_some((key.as_str(), default.as_str(), value.as_str()))
            })
            .collect()
    }

    pub fn requires_nr(&self) -> bool {
        ["nrEnabled", "rtx_nr"]
            .iter()
            .any(|key| self.values.get(*key).is_some_and(|v| v == "1"))
    }
    pub fn requires_xemfg(&self) -> bool {
        self.values.get("rtx_xemfg").is_some_and(|v| v == "1")
    }
}

pub fn capture(
    target: &core::DeploymentTarget,
    catalog: &cloud::Catalog,
    scheme: &str,
    version: &str,
) -> Result<Snapshot> {
    core::assert_target_stopped(target)?;
    let (installed, current) = presets::inspect_at(target, catalog)?
        .context("无法读取已安装方案的参数，请先应用参数再分享")?;
    ensure!(installed == scheme, "当前安装方案已变化，请重新读取");
    let policy = catalog.scheme_policies.get(scheme).context("未知方案")?;
    let context = presets::Context::new(scheme, policy, &target.game_exe);
    let mut defaults = presets::defaults(&context.profile, &policy.defaults);
    context.normalize(&mut defaults);
    defaults.retain(|key, _| supported(&context.profile, key, version));
    let mut values = defaults.clone();
    values.extend(
        current
            .into_iter()
            .filter(|(key, _)| defaults.contains_key(key)),
    );
    // A malformed or conflicting value must be visible to the publisher, never
    // converted into an apparently verified player recommendation.
    let snapshot = Snapshot {
        schema: 1,
        profile: context.profile,
        values,
        defaults,
    };
    snapshot.validate(scheme, version)?;
    Ok(snapshot)
}

/// Generated backend allowlist; checked against the client's actual metadata.
pub fn numeric_range(profile: &str, key: &str) -> Option<(f64, f64)> {
    if profile == encore::PROFILE {
        let f = encore::field(key)?;
        if matches!(
            f.kind,
            encore::FieldKind::Integer
                | encore::FieldKind::Decimal
                | encore::FieldKind::InheritDecimal
        ) {
            return f.min.zip(f.max).filter(|(a, b)| a < b);
        }
    } else if profile == rtxmfg::PROFILE {
        return match key {
            "rtx_target" | "rtx_reflex_limit" | "rtx_xemfg_target" => Some((0., 1000.)),
            "rtx_nr_scale" => Some((0., 200.)),
            "rtx_nr_intensity" => Some((0., 1.)),
            _ => None,
        };
    }
    None
}
pub fn contracts() -> Value {
    let mut result = serde_json::Map::new();
    for &(scheme, profile) in PROFILES {
        let mut fields = serde_json::Map::new();
        for p in presets::parameters(profile)
            .into_iter()
            .filter(|p| shareable(profile, p.key))
        {
            let mut rule = json!({"values": p.choices.iter().map(|(v,_)| *v).collect::<Vec<_>>()});
            if profile == encore::PROFILE {
                let f = encore::field(p.key).unwrap();
                rule = match f.kind {
                    encore::FieldKind::Bool => json!({"values":["0","1"]}),
                    encore::FieldKind::InheritBool => json!({"values":["inherit","0","1"]}),
                    encore::FieldKind::Choice => rule,
                    _ => json!({"number":true,"min":f.min,"max":f.max,
                        "integer": f.kind == encore::FieldKind::Integer,
                        "inherit": f.kind == encore::FieldKind::InheritDecimal}),
                };
                if let Some(since) = &f.since {
                    rule["since"] = json!(since);
                }
                if p.key == "nrEngine" || p.key.starts_with("nrOpen") {
                    rule["until"] = json!("1.3.0");
                }
                if p.key == "smoothMotionForce" {
                    rule["until"] = json!("1.2.0-beta.1");
                }
            } else if profile == rtxmfg::PROFILE {
                if matches!(
                    p.key,
                    "rtx_target" | "rtx_reflex_limit" | "rtx_xemfg_target"
                ) {
                    rule = json!({"number":true,"min":0,"max":1000,"integer":true});
                } else if matches!(p.key, "rtx_nr_scale" | "rtx_nr_intensity") {
                    rule = json!({"number":true,"min":0,"max":if p.key == "rtx_nr_scale" {200} else {1}});
                }
                if rtxmfg::is_170_key(p.key) {
                    rule["since"] = json!("1.7.0");
                } else if rtxmfg::is_sidecar_key(p.key)
                    || matches!(p.key, "rtx_sr_preset" | "rtx_rr_preset")
                {
                    rule["since"] = json!("1.6.0");
                }
            }
            rule["label"] = json!(p.label);
            let mut labels = p
                .choices
                .iter()
                .map(|(v, label)| ((*v).to_owned(), json!(label)))
                .collect::<serde_json::Map<_, _>>();
            if profile == encore::PROFILE
                && encore::field(p.key).is_some_and(|f| {
                    matches!(
                        f.kind,
                        encore::FieldKind::Bool | encore::FieldKind::InheritBool
                    )
                })
            {
                labels.insert("0".into(), json!("关闭"));
                labels.insert("1".into(), json!("开启"));
                labels.insert("inherit".into(), json!("跟随游戏"));
            }
            rule["labels"] = json!(labels);
            fields.insert(p.key.into(), rule);
        }
        result.insert(scheme.into(), json!({"profile":profile,"fields":fields}));
    }
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_fields_and_other_protocols_are_rejected() {
        let mut s = Snapshot {
            schema: 1,
            profile: encore::PROFILE.into(),
            values: Values::from([("tf_mode".into(), "fixed3".into())]),
            defaults: Values::from([("tf_mode".into(), "game".into())]),
        };
        // Use known metadata choices to avoid duplicating upstream enums.
        s.values.insert(
            "tf_mode".into(),
            encore::field("tf_mode").unwrap().choices[0].value.clone(),
        );
        s.defaults = s.values.clone();
        assert!(s.validate(PROFILES[0].0, "1.4.2").is_ok());
        for key in [
            "hotkeyToggleNr",
            "logDirectory",
            "unknown",
            "logNr",
            "overlayFontSize",
        ] {
            s.values.insert(key.into(), "1".into());
            s.defaults.insert(key.into(), "0".into());
            assert!(s.validate(PROFILES[0].0, "1.4.2").is_err());
            s.values.remove(key);
            s.defaults.remove(key);
        }
        assert!(s.validate(PROFILES[1].0, "1.7.0").is_err());
    }
    #[test]
    fn compare_only_changes_and_reject_retired_fields() {
        let mut s = Snapshot {
            schema: 1,
            profile: encore::PROFILE.into(),
            values: Values::from([
                ("nrIntensity".into(), "0.5".into()),
                ("dlssCustomScale".into(), "67.0".into()),
            ]),
            defaults: Values::from([
                ("nrIntensity".into(), "1".into()),
                ("dlssCustomScale".into(), "67".into()),
            ]),
        };
        assert_eq!(s.changes(), vec![("nrIntensity", "1", "0.5")]);
        s.values.insert("nrEngine".into(), "nvidia".into());
        s.defaults.insert("nrEngine".into(), "nvidia".into());
        assert!(s.validate(PROFILES[0].0, "1.4.2").is_err());
    }
    #[test]
    fn backend_contract_matches_client_fields() {
        let expected: Value = serde_json::from_str(include_str!(
            "../../recommendations/src/preset-contracts.json"
        ))
        .unwrap();
        assert_eq!(contracts(), expected);
    }
    #[test]
    #[ignore = "explicit maintainer regeneration of the public backend allowlist"]
    fn export_backend_contract() {
        std::fs::write(
            std::env::var("RTXFG_PRESET_CONTRACT_OUT").expect("explicit output path"),
            serde_json::to_vec_pretty(&contracts()).unwrap(),
        )
        .unwrap();
    }
}
