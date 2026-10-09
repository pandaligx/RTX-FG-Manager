//! Metadata-backed Encore editor. Draft input belongs to this game only; valid
//! edits use Controller's per-field dirty tracking, never a full snapshot write.
use super::*;
use rtx_fg_manager::{encore, presets::Values};
use std::collections::{BTreeMap, BTreeSet};

struct TextBinding {
    state: Entity<InputState>,
    _subscription: Subscription,
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
        let search = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_lowercase();
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
            _subscriptions: subscriptions,
        }
    }

    fn t(&self, text: &str) -> SharedString {
        self.tr.t(text).into()
    }

    fn set(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        let exe = self.exe.clone();
        let _ = self.owner.update(cx, |owner, cx| {
            if owner.c.focus.as_deref() == Some(&exe)
                && owner.c.parameter_profile() == encore::PROFILE
            {
                owner.c.set_game_option(&exe, key, value);
                cx.notify();
            }
        });
        self.errors.remove(key);
        self.invalid.set(!self.errors.is_empty());
        cx.notify();
    }

    fn reset(&mut self, group_only: bool, cx: &mut Context<Self>) {
        let defaults = encore::defaults();
        let selected: Vec<_> = encore::fields()
            .iter()
            .filter(|field| !group_only || field.group == self.group)
            .map(|field| field.key.clone())
            .collect();
        for key in selected {
            self.set(&key, &defaults[&key], cx);
        }
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
                    .xsmall()
                    .ghost()
                    .label(self.t("还原"))
                    .tooltip(self.t("恢复此项默认值"))
                    .disabled(locked)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set(&key, &field.default, cx);
                    })),
            );
        if field.restart {
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
                .checked(value == "1")
                .disabled(disabled)
                .label(self.t(if value == "1" { "开启" } else { "关闭" }))
                .on_click(cx.listener(move |this, yes, _, cx| {
                    this.set(&key, if *yes { "1" } else { "0" }, cx);
                }))
                .into_any_element()
        } else if matches!(field.kind, encore::FieldKind::Choice) {
            let selected = field
                .choices
                .iter()
                .find(|c| &c.value == value)
                .map(|c| self.t(&c.label))
                .unwrap_or_else(|| SharedString::from(value.clone()));
            let choices: Vec<_> = field
                .choices
                .iter()
                .map(|choice| (choice.value.clone(), self.t(&choice.label)))
                .collect();
            let owner = cx.weak_entity();
            let key = field.key.clone();
            Button::new(SharedString::from(format!("encore-{}", field.key)))
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
                        let unsupported = key == "nrEngine" && value == "opendlss" && gpu == 0;
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .disabled(unsupported)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| this.set(&key, &value, cx));
                                }),
                        );
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
        if !field.choices.is_empty() && !matches!(field.kind, encore::FieldKind::Choice) {
            let mut quick = div().h_flex().flex_wrap().gap_1();
            for (index, choice) in field.choices.iter().enumerate() {
                let key = field.key.clone();
                let value = choice.value.clone();
                quick = quick.child(
                    Button::new((SharedString::from(format!("quick-{}", key)), index))
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
                    .child(self.t("需先启用相关功能，并使用兼容的显卡系列。")),
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
        let locked = c.busy
            || c.read_only
            || c.running_games.contains(&self.exe)
            || c.preset_reads.contains(&self.exe);
        let gpu = c.cloud_series().min(2) as i32;
        let can_apply = c.can_apply_parameters();
        let dirty = c.preset_dirty(&self.exe);
        let upgrade = c
            .deployments
            .get(&self.exe)
            .is_some_and(|d| d.incompatible_protocol);
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
                    .small()
                    .label(self.t(title))
                    .when(group == self.group, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = group;
                        cx.notify();
                    })),
            );
        }
        let mut panel = div().v_flex().w_full().min_w_0().gap_3()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("RTX Encore · 1.0.0-beta.2"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(self.t("游戏内按Insert打开菜单；首次启动自动显示。管理器中的修改须完全退出游戏后应用。")))
            .child(div().text_xs().child(self.t("搜索参数名称或说明")))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(self.t("多目录游戏显示首个实际部署目录的参数；仅将明确修改项应用到每个目录。")))
            .child(Input::new(&self.search).w_full())
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
            .filter(|field| {
                if self.query.is_empty() {
                    field.group == self.group
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
        panel = panel.child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("encore-reset-group")
                        .small()
                        .ghost()
                        .label(self.t("恢复本组默认"))
                        .disabled(locked || !self.query.is_empty())
                        .tooltip(self.t("清除搜索后，可恢复当前分组默认值。"))
                        .on_click(cx.listener(|this, _, _, cx| this.reset(true, cx))),
                )
                .child(
                    Button::new("encore-reset-all")
                        .small()
                        .ghost()
                        .label(self.t("恢复全部默认"))
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, _, cx| this.reset(false, cx))),
                ),
        );
        if dirty {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().info)
                    .child(self.t("有修改，待应用")),
            );
        }
        if !self.errors.is_empty() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(self.t("请先修正高级参数中的无效输入，或恢复该项默认值。")),
            );
        }
        if can_apply {
            panel = panel.child(
                Button::new("encore-apply")
                    .small()
                    .primary()
                    .label(self.t("应用参数到当前游戏"))
                    .disabled(locked || !self.errors.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        let _ = this
                            .owner
                            .update(cx, |owner, cx| owner.act(Command::ApplyPreset, window, cx));
                    })),
            );
        }
        panel.into_any_element()
    }
}
