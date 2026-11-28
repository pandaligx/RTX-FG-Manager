//! Per-install components and first-install engines, separate from the preset editor.
use super::*;
use crate::controller::{DlssgInstallOption, NativeSelections};
use rtx_fg_manager::encore_install::{Frames, Reconstruction};
use rtx_fg_manager::runtime_files::{NativeComponent, label as runtime_label};
impl Manager {
    pub(super) fn patch_install_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = &self.c.patch_install_plan else {
            self.c.confirm = None;
            return;
        };
        let choices = Rc::new(RefCell::new(plan.choices.clone()));
        let dlssg_options = plan.dlssg_options.clone();
        let opti_options = plan.opti_options.clone();
        let selected_native = Rc::new(RefCell::new(NativeSelections::default()));
        let title = match (
            plan.choices.is_empty(),
            dlssg_options.is_empty(),
            opti_options.is_empty(),
        ) {
            (true, false, true) => "Dlssg_for_sm86 安装选项",
            (false, true, true) => "Encore 安装选项",
            _ => "补丁安装选项",
        };
        let owner = cx.entity().downgrade();
        let language = self.c.tr.language.clone();
        let width = (window.viewport_size().width - px(60.)).min(window.rem_size() * 49.);
        let height = (window.viewport_size().height - window.rem_size() * 16.).max(px(150.));
        self.c.confirm = None;
        self.dialog_active = true;
        window.open_dialog(cx,move|dialog,_,cx|{
            let tr=i18n::Translator::new(&language);
            let mut body=div().id("patch-install-plan").v_flex().gap_3().max_h(height).overflow_y_scroll().text_sm();
            for (index,choice) in choices.borrow().iter().enumerate() {
                let mut row=div().v_flex().gap_2().pb_3().border_b_1().border_color(cx.theme().border)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(choice.name.clone()))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).whitespace_normal().child(choice.exe.clone()));
                if choice.needs_frames {
                    let mut buttons=div().h_flex().flex_wrap().gap_1();
                    for (n,frame,label) in [(0,Frames::None,"不启用"),(1,Frames::SmoothMotion,"Smooth Motion"),(2,Frames::Dlss,"DLSS-G"),(3,Frames::Xefg,"XeFG")] {
                        let selections=choices.clone();
                        buttons=buttons.child(Button::new(SharedString::from(format!("engine-fg-{index}-{n}"))).small()
                            .focus_ring(crate::ui_focus::visible(cx)).label(tr.t(label)).selected(choice.frames==Some(frame))
                            .disabled(frame==Frames::Dlss&&!choice.dlss_fg_available)
                            .on_click(move|_,window,_|{selections.borrow_mut()[index].frames=Some(frame);window.refresh();}));
                    }
                    row=row.child(tr.t("帧生成")).child(buttons);
                    if !choice.dlss_fg_available {row=row.child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr.t("检测到原生帧生成或文件冲突，新增 DLSS-G 暂不可用；保留原文件，可选择其他引擎。")));}
                } else {row=row.child(div().text_xs().child(tr.t("保留游戏原生帧生成")));}
                if choice.needs_reconstruction {
                    let mut buttons=div().h_flex().flex_wrap().gap_1();
                    for (n,sr,label) in [(0,Reconstruction::None,"不启用"),(1,Reconstruction::Dlss,"DLSS"),(2,Reconstruction::Xess,"XeSS"),(5,Reconstruction::Mwsr,"MWSR"),(3,Reconstruction::Nis,"NIS"),(4,Reconstruction::Fsr1,"FSR 1")] {
                        if sr == Reconstruction::Mwsr && !choice.mwsr_available { continue; }
                        if sr == Reconstruction::Fsr1 && !choice.fsr1_available { continue; }
                        if sr == Reconstruction::Nis && !choice.nis_available { continue; }
                        let selections=choices.clone();
                        buttons=buttons.child(Button::new(SharedString::from(format!("engine-sr-{index}-{n}"))).small()
                            .focus_ring(crate::ui_focus::visible(cx)).label(tr.t(label)).selected(choice.reconstruction==Some(sr))
                            .on_click(move|_,window,_|{selections.borrow_mut()[index].reconstruction=Some(sr);window.refresh();}));
                    }
                    let selections=choices.clone();
                    row=row.child(tr.t("超分辨率")).child(buttons)
                        .child(Checkbox::new(SharedString::from(format!("engine-screen-{index}"))).label(tr.t("放大到屏幕分辨率（默认使用游戏分辨率）"))
                            .checked(choice.screen).disabled(choice.reconstruction.is_none_or(|r|r==Reconstruction::None))
                            .on_click(move|checked,window,_|{selections.borrow_mut()[index].screen = *checked;window.refresh();}));
                } else {row=row.child(div().text_xs().child(tr.t("保留游戏原生超分辨率")));}
                row = row.child(div().pt_2().font_weight(FontWeight::SEMIBOLD).child(tr.t("安装组件")));
                for (kind, label, description, enabled) in [
                    ("patch", "RTX Encore", "补丁与设置文件", choice.installed),
                    ("sr", "DLSS 超分辨率", "nvngx_dlss.dll  310.9.16", true),
                    ("rr", "DLSS 光线重建", "nvngx_dlssd.dll  310.9.16", true),
                    ("fg", "DLSS 帧生成", "nvngx_dlssg.dll  310.9.16", choice.native_fg),
                    ("nr", "DLSS 神经渲染", "nvngx_dlssnr.dll  310.9.16", true),
                    ("intel", "Intel XeSS / XeFG / XeLL", "libxess.dll / libxess_fg.dll / libxell.dll", true),
                    ("streamline", "更新已有 Streamline 插件至 2.14.16", "默认保留现有插件及 sl.interposer.dll", choice.native_streamline),
                    ("encorefg", "新增 DLSS 帧生成组件", "无原生帧生成时使用，包含匹配的 Streamline 文件", choice.dlss_fg_available),
                ] {
                    let selections = choices.clone();
                    row = row.child(div().h_flex().items_start().gap_2()
                        .child(div().w(rems(22.)).flex_shrink_0().child(Checkbox::new(SharedString::from(format!("component-{index}-{kind}")))
                            .checked(choice.components.selected(kind)).disabled(!enabled)
                            .label(tr.t(label))
                            .on_click(move |checked, window, _| { selections.borrow_mut()[index].components.set(kind, *checked); window.refresh(); })))
                        .child(div().flex_1().min_w_0().text_xs().text_color(cx.theme().muted_foreground).child(tr.t(description))));
                }
                if choice.components.streamline {
                    row = row.child(div().text_xs().text_color(cx.theme().warning).child(tr.t("部分游戏需要原版 Streamline。仅更新检测到的插件，原始 interposer 保留。")));
                }
                body=body.child(row);
            }
            for (index, option) in dlssg_options.iter().enumerate() {
                body = body.child(dlssg_components(option, index, selected_native.clone(), &tr, cx));
            }
            for (index, (exe, game)) in opti_options.iter().enumerate() {
                let mut row = div().v_flex().gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(game.clone()))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(exe.clone()));
                let state = selected_native.clone();
                let exe = exe.clone();
                let checked = state.borrow().opti_nr.contains(&exe);
                row = row.child(Checkbox::new(("opti-nr", index))
                    .checked(checked).label(format!("{}  310.9.16", tr.t("DLSS 神经渲染")))
                    .on_click(move |on, window, _| {
                        let mut selected = state.borrow_mut();
                        if *on { selected.opti_nr.insert(exe.clone()); } else { selected.opti_nr.remove(&exe); }
                        window.refresh();
                    }));
                body = body.child(row);
            }
            let footer_hint = if !opti_options.is_empty() && dlssg_options.is_empty() && choices.borrow().is_empty() {
                tr.t("已包含 OptiScaler 专用 Streamline 2.14.16。")
            } else if dlssg_options.is_empty() {
                tr.t("未勾选的现有文件保持不变。恢复原版请使用运行库的恢复按钮。")
            } else {
                let tip = tr.t("默认不勾选，如部分老游戏不生效，有需要时再勾选以上组件。");
                if choices.borrow().is_empty() { tip } else { format!("Dlssg_for_sm86 — {tip}") }
            };
            body=body.child(div().text_xs().text_color(cx.theme().muted_foreground).child(footer_hint));
            let ready=choices.borrow().iter().all(|c|c.values().is_some());
            if !ready { body=body.child(div().text_xs().text_color(cx.theme().warning).child(tr.t("请选择引擎，并勾选其所需组件；也可选择不启用。"))); }
            let accept=owner.clone();let cancel=owner.clone();let selections=choices.clone();
            let selected_native = selected_native.clone();
            dialog.title(crate::ui_focus::dialog_title(tr.t(title).into(),cx)).width(width).close_button(false)
                .child(body)
                .footer(DialogFooter::new()
                    .child(Button::new("engines-cancel").small().label(tr.t("取消")).on_click(|_,w,cx|w.dispatch_action(Box::new(Cancel),cx)))
                    .child(Button::new("engines-install").small().primary().disabled(!ready).label(tr.t("安装并应用"))
                        .on_click(|_,w,cx|w.dispatch_action(Box::new(Confirm{secondary:false}),cx))))
                .on_ok(move|_,_,cx|{
                    if !selections.borrow().iter().all(|c|c.values().is_some()){return false;}
                    let _=accept.update(cx,|owner,cx|{owner.dialog_active=false;
                        if let Err(e)=owner.c.confirm_patch_options(selections.borrow().clone(), selected_native.borrow().clone()){owner.c.warning=Some(e.to_string());}
                        cx.notify();});true
                })
                .on_cancel(move|_,_,cx|{let _=cancel.update(cx,|owner,cx|{owner.dialog_active=false;owner.c.cancel_patch();cx.notify();});true})
        });
    }
}

fn dlssg_components(
    option: &DlssgInstallOption,
    index: usize,
    selected: Rc<RefCell<NativeSelections>>,
    tr: &i18n::Translator,
    cx: &App,
) -> impl IntoElement {
    let row = |check: Checkbox, detail: String| {
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .child(div().w(rems(22.)).flex_shrink_0().child(check))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
    };
    let mut body = div()
        .v_flex()
        .gap_2()
        .pb_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(option.game.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(option.exe.clone()),
        )
        .child(
            div()
                .pt_2()
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr.t("安装组件")),
        )
        .child(row(
            Checkbox::new(("dlssg-patch", index))
                .checked(true)
                .disabled(true)
                .label(option.patch_label.clone()),
            tr.t("补丁与设置文件"),
        ));
    for kind in NativeComponent::ALL {
        let exe = option.exe.clone();
        let selection = selected.clone();
        let checked = selection
            .borrow()
            .runtimes
            .get(&exe)
            .is_some_and(|set| set.contains(&kind));
        let reason = option.unavailable.get(&kind).map(|reason| tr.t(reason));
        let check = Checkbox::new((kind.kind(), index))
            .checked(checked)
            .disabled(reason.is_some())
            .label(tr.t(runtime_label(kind.kind())))
            .on_click(move |checked, window, _| {
                let all = &mut selection.borrow_mut().runtimes;
                if *checked {
                    all.entry(exe.clone()).or_default().insert(kind);
                } else if let Some(set) = all.get_mut(&exe) {
                    set.remove(&kind);
                    if set.is_empty() {
                        all.remove(&exe);
                    }
                }
                window.refresh();
            });
        let detail = match kind {
            NativeComponent::SuperResolution => "nvngx_dlss.dll",
            NativeComponent::RayReconstruction => "nvngx_dlssd.dll",
            NativeComponent::FrameGeneration => "nvngx_dlssg.dll",
            NativeComponent::Streamline => "Streamline SDK",
        };
        body = body.child(
            row(check, format!("{detail}  {}", kind.version()))
                .id(SharedString::from(format!("dlssg-{index}-{}", kind.kind())))
                .when_some(reason, |r, reason| {
                    r.tooltip(move |w, cx| Tooltip::new(reason.clone()).build(w, cx))
                }),
        );
    }
    if let Some(enabled) = option.yanyun {
        let exe = option.exe.clone();
        let checked = selected.borrow().yanyun.contains(&exe);
        body = body.child(row(
            Checkbox::new(("yanyun-nr", index))
                .label(tr.t("燕云十六声 DLSS5 神经渲染套件"))
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |on, window, _| {
                    if *on {
                        selected.borrow_mut().yanyun.insert(exe.clone());
                    } else {
                        selected.borrow_mut().yanyun.remove(&exe);
                    }
                    window.refresh();
                }),
            tr.t(if enabled {
                "在游戏画面设置中开启"
            } else {
                "帧生成入口请只选 version.dll"
            }),
        ));
    }
    body.when(option.miles_profile, |view| view.child(
        div().text_xs().whitespace_normal().text_color(cx.theme().muted_foreground)
            .child(tr.t("《迈尔斯》要使用 4X 或 6X DLSS 帧生成，请同时勾选「DLSS 帧生成」和「Streamline 原生插件」。卸载时会还原游戏原件。"))
    ))
}
