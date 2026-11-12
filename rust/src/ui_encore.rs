//! Metadata-backed Encore editor. Draft input belongs to this game only; valid
//! edits use Controller's per-field dirty tracking, never a full snapshot write.
use super::*;
use rtx_fg_manager::{encore, presets::Values};
use std::collections::{BTreeMap, BTreeSet};

struct TextBinding {
    state: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Clone, Copy)]
enum AddedFeature {
    Sr,
    Nr,
    Fg,
}
impl AddedFeature {
    fn key(self) -> &'static str {
        match self {
            Self::Sr => "dlssWithoutGameDlss",
            Self::Nr => "nrWithoutGameDlss",
            Self::Fg => "frameGenerationWithoutGameFg",
        }
    }
    fn engine(self) -> Option<&'static str> {
        match self {
            Self::Sr => Some("dlssWithoutGameDlssEngine"),
            Self::Fg => Some("frameGenerationEngine"),
            Self::Nr => None,
        }
    }
    fn selected(self, values: &Values) -> &str {
        if values.get(self.key()).is_none_or(|v| v != "1") {
            return "off";
        }
        match self {
            Self::Sr
                if values
                    .get("dlssWithoutGameDlssEngine")
                    .is_some_and(|v| v == "xess") =>
            {
                "xess"
            }
            Self::Sr
                if values
                    .get("dlssWithoutGameDlssEngine")
                    .is_some_and(|v| v == "nis") =>
            {
                "nis"
            }
            Self::Sr
                if values
                    .get("dlssWithoutGameDlssEngine")
                    .is_some_and(|v| v == "fsr1") =>
            {
                "fsr1"
            }
            Self::Sr => "dlss",
            Self::Fg
                if values
                    .get("frameGenerationEngine")
                    .is_some_and(|v| v == "xefg") =>
            {
                "xefg"
            }
            Self::Fg => "dlss-fg",
            Self::Nr => "on",
        }
    }
    fn edits(self, value: &str) -> Values {
        let mut edits = Values::from([(
            self.key().into(),
            if value == "off" { "0" } else { "1" }.into(),
        )]);
        if value != "off"
            && let Some(engine) = self.engine()
        {
            edits.insert(engine.into(), value.into());
        }
        edits
    }
}

fn in_added_section(key: &str, values: &Values) -> bool {
    matches!(
        key,
        "dlssWithoutGameDlss"
            | "dlssWithoutGameDlssEngine"
            | "nrWithoutGameDlss"
            | "frameGenerationWithoutGameFg"
    ) || key == "frameGenerationEngine" && AddedFeature::Fg.selected(values) != "off"
}

pub(super) struct EncoreEditor {
    owner: WeakEntity<Manager>,
    exe: String,
    group: &'static str,
    search: Entity<InputState>,
    query: String,
    inputs: HashMap<String, TextBinding>,
    errors: BTreeSet<String>,
    invalid: Rc<Cell<bool>>,
    tr: i18n::Translator,
    version: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EncoreEditor {
    pub(super) fn new(
        exe: String,
        owner: WeakEntity<Manager>,
        invalid: Rc<Cell<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::ui_focus::observe(cx);
        let search = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_lowercase();
                let _ = this.owner.update(cx, |_, cx| cx.notify());
                cx.notify();
            }
        });
        let mut subscriptions = vec![subscription];
        if let Some(parent) = owner.upgrade() {
            subscriptions.push(cx.observe(&parent, |_, _, cx| cx.notify()));
        }
        Self {
            owner,
            exe,
            group: "frames",
            search,
            query: String::new(),
            inputs: HashMap::new(),
            errors: BTreeSet::new(),
            invalid,
            tr: i18n::Translator::new("zh-CN"),
            version: None,
            _subscriptions: subscriptions,
        }
    }

    fn t(&self, text: &str) -> SharedString {
        self.tr.t(text).into()
    }

    fn set(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        self.set_edits(&Values::from([(key.into(), value.into())]), cx);
    }

    fn set_edits(&mut self, edits: &Values, cx: &mut Context<Self>) {
        let exe = self.exe.clone();
        let _ = self.owner.update(cx, |owner, cx| {
            if owner.c.focus.as_deref() == Some(&exe)
                && owner.c.parameter_profile() == encore::PROFILE
            {
                owner.c.set_game_options(&exe, edits);
                cx.notify();
            }
        });
        for key in edits.keys() {
            self.errors.remove(key);
        }
        if edits.get("nrFoveaPreset").is_some_and(|v| v != "custom") {
            for key in [
                "nrFoveaWidth",
                "nrFoveaHeight",
                "nrFoveaFalloff",
                "nrFoveaEdge",
            ] {
                self.errors.remove(key);
                self.inputs.remove(key);
            }
        }
        self.invalid.set(!self.errors.is_empty());
        cx.notify();
    }

    fn reset(&mut self, group_only: bool, cx: &mut Context<Self>) {
        let mut defaults = encore::defaults_for_version(self.version.as_deref());
        encore::localize_defaults(&mut defaults, &self.tr.language);
        let current = self
            .owner
            .upgrade()
            .map(|owner| owner.read(cx).c.preset_values(&self.exe))
            .unwrap_or_else(|| defaults.clone());
        let edits: Values = encore::fields()
            .iter()
            .filter(|field| encore::field_available(field, self.version.as_deref()))
            .filter(|field| !group_only || field.group == self.group)
            .filter(|field| !group_only || !in_added_section(&field.key, &current))
            .map(|field| (field.key.clone(), defaults[&field.key].clone()))
            .collect();
        self.set_edits(&edits, cx);
    }

    fn added_section(
        &self,
        values: &Values,
        installed: Option<&rtx_fg_manager::deployment::Snapshot>,
        locked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = div().v_flex().gap_2().min_w_0();
        for (feature, label, mut choices) in [
            (
                AddedFeature::Sr,
                "新增超分辨率",
                vec![("off", "关闭"), ("dlss", "DLSS"), ("xess", "XeSS")],
            ),
            (
                AddedFeature::Nr,
                "新增神经渲染",
                vec![("off", "关闭"), ("on", "DLSSNR")],
            ),
            (
                AddedFeature::Fg,
                "新增帧生成",
                vec![("off", "关闭"), ("dlss-fg", "DLSS-FG"), ("xefg", "XeFG")],
            ),
        ] {
            if matches!(feature, AddedFeature::Sr) && encore::supports_220(self.version.as_deref())
            {
                choices.push(("nis", "NIS"));
            }
            if matches!(feature, AddedFeature::Sr) && encore::supports_230(self.version.as_deref())
            {
                choices.push(("fsr1", "FSR 1"));
            }
            let selected = feature.selected(values);
            let choice_label = choices
                .iter()
                .find(|(v, _)| *v == selected)
                .map(|(_, l)| *l)
                .unwrap_or("关闭");
            let owner = cx.weak_entity();
            let choices: Vec<_> = choices.into_iter().map(|(v, l)| (v, self.t(l))).collect();
            let selected = selected.to_owned();
            let ready = installed.is_some_and(|s| {
                let statuses: Vec<_> = s
                    .runtimes
                    .iter()
                    .filter(|r| {
                        r.kind
                            == match feature {
                                AddedFeature::Sr => "sr",
                                AddedFeature::Fg => "encorefg",
                                AddedFeature::Nr => "",
                            }
                    })
                    .collect();
                match feature {
                    AddedFeature::Nr => {
                        !s.neural.is_empty()
                            && s.neural.iter().all(|n| n.present && n.warning.is_none())
                    }
                    _ => !statuses.is_empty() && statuses.iter().all(|r| r.present),
                }
            });
            let message = if selected == "off" {
                ""
            } else if installed.is_none_or(|s| s.installed == 0) {
                "请先安装 Encore"
            } else if matches!(selected.as_str(), "nis" | "fsr1") {
                "无需额外运行库"
            } else if matches!(selected.as_str(), "xess" | "xefg") {
                "随 Encore 提供"
            } else if ready {
                "组件已就绪"
            } else {
                "需安装运行库"
            };
            let picker = Button::new(SharedString::from(format!("added-{}", feature.key())))
                .focus_ring(crate::ui_focus::visible(cx))
                .small()
                .w(rems(10.))
                .label(self.t(choice_label))
                .icon(IconName::ChevronDown)
                .disabled(locked)
                .dropdown_menu(move |mut menu, _, _| {
                    for (value, label) in &choices {
                        let owner = owner.clone();
                        let edits = feature.edits(value);
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(*value == selected)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| this.set_edits(&edits, cx));
                                }),
                        );
                    }
                    menu
                });
            rows = rows.child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .w(rems(10.))
                            .flex_shrink_0()
                            .text_sm()
                            .child(self.t(label)),
                    )
                    .child(picker)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t(message)),
                    ),
            );
        }
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.t("为游戏补充 DLSS / 帧生成")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        self.t(
                            "若游戏原本没有 DLSS 或帧生成，可使用以下功能。实验功能，默认关闭。",
                        ),
                    ),
            )
            .child(rows)
            .when(
                values
                    .get("frameGenerationWithoutGameFg")
                    .is_some_and(|v| v == "1")
                    && values.get("smoothMotionEnabled").is_some_and(|v| v == "1"),
                |view| {
                    view.child(
                        div().text_xs().text_color(cx.theme().warning).child(
                            self.t("新增帧生成不能与 Smooth Motion 同时开启，请关闭其中一项"),
                        ),
                    )
                },
            )
            .into_any_element()
    }

    fn text_input(
        &mut self,
        field: &'static encore::Field,
        value: &str,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.inputs.contains_key(&field.key) {
            let state = cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(value.to_owned(), window, cx);
                input
            });
            let key = field.key.clone();
            let subscription = cx.subscribe(&state, move |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().trim().to_owned();
                    let values = BTreeMap::from([(key.clone(), value.clone())]);
                    if encore::validate_values(&values).is_ok() {
                        this.set(&key, &value, cx);
                    } else {
                        this.errors.insert(key.clone());
                        this.invalid.set(true);
                        let _ = this.owner.update(cx, |_, cx| cx.notify());
                        cx.notify();
                    }
                }
            });
            self.inputs.insert(
                field.key.clone(),
                TextBinding {
                    state,
                    _subscription: subscription,
                },
            );
        }
        let binding = &self.inputs[&field.key];
        if !self.errors.contains(&field.key) && binding.state.read(cx).value().as_ref() != value {
            binding.state.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        }
        Input::new(&binding.state)
            .focus_ring(crate::ui_focus::visible(cx))
            .disabled(disabled)
            .w_full()
            .into_any_element()
    }

    fn field(
        &mut self,
        field: &'static encore::Field,
        values: &Values,
        locked: bool,
        gpu: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = values.get(&field.key).unwrap_or(&field.default);
        let enabled = encore::field_enabled(&field.key, values, Some(gpu));
        let disabled = locked || !enabled;
        let key = field.key.clone();
        let mut heading = div()
            .h_flex()
            .gap_2()
            .items_start()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(self.t(&field.label)),
            )
            .child(
                Button::new(SharedString::from(format!("reset-{}", field.key)))
                    .focus_ring(crate::ui_focus::visible(cx))
                    .xsmall()
                    .ghost()
                    .label(self.t("还原"))
                    .tooltip(self.t("恢复此项默认值"))
                    .disabled(locked)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let defaults = encore::defaults_for_version(this.version.as_deref());
                        this.set(&key, &defaults[&key], cx);
                    })),
            );
        if encore::requires_restart(field, self.version.as_deref()) {
            heading = heading.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("需重启")),
            );
        }
        let control = if matches!(field.kind, encore::FieldKind::Bool) {
            let key = field.key.clone();
            Switch::new(SharedString::from(format!("encore-{}", field.key)))
                .focus_ring(crate::ui_focus::visible(cx))
                .checked(value == "1")
                .disabled(disabled)
                .label(self.t(if value == "1" { "开启" } else { "关闭" }))
                .on_click(cx.listener(move |this, yes, _, cx| {
                    this.set(&key, if *yes { "1" } else { "0" }, cx);
                }))
                .into_any_element()
        } else if matches!(
            field.kind,
            encore::FieldKind::Choice | encore::FieldKind::InheritBool
        ) {
            let selected = field
                .choices
                .iter()
                .find(|c| &c.value == value)
                .map(|c| self.t(&c.label))
                .unwrap_or_else(|| SharedString::from(value.clone()));
            let choices: Vec<_> = field
                .choices
                .iter()
                .filter(|choice| encore::choice_available(choice, self.version.as_deref()))
                .map(|choice| (choice.value.clone(), self.t(&choice.label)))
                .collect();
            let owner = cx.weak_entity();
            let key = field.key.clone();
            Button::new(SharedString::from(format!("encore-{}", field.key)))
                .focus_ring(crate::ui_focus::visible(cx))
                .small()
                .w_full()
                .label(selected)
                .icon(IconName::ChevronDown)
                .disabled(disabled)
                .dropdown_menu(move |mut menu, window, _| {
                    menu = menu
                        .max_h((window.viewport_size().height - px(200.)).max(px(120.)))
                        .max_w(px(420.))
                        .scrollable(true);
                    for (value, label) in &choices {
                        let owner = owner.clone();
                        let key = key.clone();
                        let value = value.clone();
                        menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                            move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set(&key, &value, cx));
                            },
                        ));
                    }
                    menu
                })
                .into_any_element()
        } else {
            self.text_input(field, value, disabled, window, cx)
        };
        let mut row = div()
            .v_flex()
            .gap_2()
            .w_full()
            .min_w_0()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(heading)
            .child(control);
        if !field.choices.is_empty()
            && !matches!(
                field.kind,
                encore::FieldKind::Choice | encore::FieldKind::InheritBool
            )
        {
            let mut quick = div().h_flex().flex_wrap().gap_1();
            for (index, choice) in field.choices.iter().enumerate() {
                let key = field.key.clone();
                let value = choice.value.clone();
                quick = quick.child(
                    Button::new((SharedString::from(format!("quick-{}", key)), index))
                        .focus_ring(crate::ui_focus::visible(cx))
                        .xsmall()
                        .ghost()
                        .label(self.t(&choice.label))
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| this.set(&key, &value, cx))),
                );
            }
            row = row.child(quick);
        }
        if !field.help.is_empty() {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(&field.help)),
            );
        }
        if field.key == "nrEngine" {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Open引擎不支持RTX20；RTX30/40仍为高度实验功能。")),
            );
        }
        if let (Some(min), Some(max)) = (field.min, field.max) {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(&format!("允许范围：{min} — {max}"))),
            );
        }
        if !enabled {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("请先开启所需功能，并满足该参数的启用条件。")),
            );
        }
        if self.errors.contains(&field.key) {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(self.t("输入无效，请按范围和说明填写。此项尚未保存。")),
            );
        }
        row.into_any_element()
    }
}

impl Render for EncoreEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let c = &owner.read(cx).c;
        if c.focus.as_deref() != Some(&self.exe) || c.parameter_profile() != encore::PROFILE {
            return div().into_any_element();
        }
        let language = c.tr.language.clone();
        let values = c.preset_values(&self.exe);
        let installed = c.deployments.get(&self.exe);
        self.version = if installed.is_some_and(|s| s.installed > 0) {
            installed.and_then(|s| {
                s.neural
                    .iter()
                    .filter_map(|n| n.encore_version.as_ref())
                    .filter_map(|v| semver::Version::parse(v).ok().map(|n| (n, v)))
                    .min_by(|a, b| a.0.cmp(&b.0))
                    .map(|(_, v)| v.clone())
            })
        } else {
            let p = c.catalog.selected(encore::SCHEME);
            Some(p.upstream_version.as_ref().unwrap_or(&p.version).clone())
        };
        let locked = c.busy
            || c.read_only
            || c.running_games.contains(&self.exe)
            || c.preset_reads.contains(&self.exe);
        let gpu = c.cloud_series().min(2) as i32;
        let upgrade = c
            .deployments
            .get(&self.exe)
            .is_some_and(|d| d.incompatible_protocol);
        let installed = installed.cloned();
        if self.tr.language != language {
            self.tr = i18n::Translator::new(&language);
        }
        let mut groups = div().h_flex().flex_wrap().gap_1();
        for (group, title) in [
            ("frames", "帧生成"),
            ("image", "画质"),
            ("overlay", "菜单与叠加层"),
            ("system", "兼容与热键"),
        ] {
            groups = groups.child(
                Button::new(group)
                    .focus_ring(crate::ui_focus::visible(cx))
                    .small()
                    .label(self.t(title))
                    .when(group == self.group, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = group;
                        let _ = this.owner.update(cx, |_, cx| cx.notify());
                        cx.notify();
                    })),
            );
        }
        let added = encore::supports_200(self.version.as_deref());
        let mut panel = div().v_flex().w_full().min_w_0().gap_3();
        if added && self.query.is_empty() {
            panel = panel.child(self.added_section(&values, installed.as_ref(), locked, cx));
        }
        panel = panel
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(self.t("游戏内默认按 Insert 打开菜单；禁用游戏内菜单后不可呼出。管理器中的修改须完全退出游戏后应用。")))
            .child(div().text_xs().child(self.t("搜索参数名称或说明")))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(self.t("多目录游戏显示首个实际部署目录的参数；仅将明确修改项应用到每个目录。")))
            .child(Input::new(&self.search).focus_ring(crate::ui_focus::visible(cx)).w_full())
            .child(groups);
        if upgrade {
            panel = panel.child(
                Alert::info(
                    "encore-upgrade-needed",
                    self.t(
                        "当前仍为Transfusion。点击安装并应用升级为Encore后，才能直接应用新参数。",
                    ),
                )
                .small(),
            );
        }
        let fields: Vec<_> = encore::fields()
            .iter()
            .filter(|field| encore::field_available(field, self.version.as_deref()))
            .filter(|field| encore::field_visible(field, &values))
            .filter(|field| {
                if self.query.is_empty() {
                    field.group == self.group && !(added && in_added_section(&field.key, &values))
                } else {
                    format!(
                        "{} {} {}",
                        self.tr.t(&field.label),
                        self.tr.t(&field.help),
                        field.key
                    )
                    .to_lowercase()
                    .contains(&self.query)
                }
            })
            .collect();
        if fields.is_empty() {
            panel = panel.child(self.t("没有匹配的参数"));
        }
        for field in fields {
            panel = panel.child(self.field(field, &values, locked, gpu, window, cx));
        }
        if !self.errors.is_empty() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(self.t("请先修正高级参数中的无效输入，或恢复该项默认值。")),
            );
        }
        panel.into_any_element()
    }
}

impl EncoreEditor {
    pub(super) fn reset_buttons(
        editor: &Entity<Self>,
        locked: bool,
        tr: &i18n::Translator,
        cx: &App,
    ) -> AnyElement {
        let this = editor.read(cx);
        let group = editor.clone();
        let all = editor.clone();
        div()
            .h_flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("encore-reset-group")
                    .focus_ring(crate::ui_focus::visible(cx))
                    .small()
                    .ghost()
                    .label(tr.t("恢复本组默认"))
                    .disabled(locked || !this.query.is_empty())
                    .tooltip(tr.t("清除搜索后，可恢复当前分组默认值。"))
                    .on_click(move |_, _, cx| group.update(cx, |this, cx| this.reset(true, cx))),
            )
            .child(
                Button::new("encore-reset-all")
                    .focus_ring(crate::ui_focus::visible(cx))
                    .small()
                    .ghost()
                    .label(tr.t("恢复全部默认"))
                    .disabled(locked)
                    .on_click(move |_, _, cx| all.update(cx, |this, cx| this.reset(false, cx))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn added_controls_map_only_their_fields_and_keep_native_choices() {
        let mut values = encore::defaults_for_version(Some("2.0.1"));
        for (feature, choice) in [
            (AddedFeature::Sr, "xess"),
            (AddedFeature::Nr, "on"),
            (AddedFeature::Fg, "xefg"),
        ] {
            assert_eq!(feature.selected(&values), "off");
            let edits = feature.edits(choice);
            encore::validate_values(&edits).unwrap();
            values.extend(edits);
            assert_eq!(feature.selected(&values), choice);
            let off = feature.edits("off");
            assert_eq!(
                off.len(),
                1,
                "turning off must not overwrite the native engine"
            );
            values.extend(off);
            assert_eq!(feature.selected(&values), "off");
        }
        assert_eq!(values["frameGenerationEngine"], "xefg");
        assert!(!in_added_section("frameGenerationEngine", &values));
        values.extend(AddedFeature::Fg.edits("dlss-fg"));
        assert!(in_added_section("frameGenerationEngine", &values));
        assert!(in_added_section("nrWithoutGameDlss", &values));
        assert!(!in_added_section("nrEnabled", &values));
    }
}
