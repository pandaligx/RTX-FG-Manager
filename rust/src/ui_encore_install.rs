//! First install selection, separate from the existing preset editor.
use super::*;
use rtx_fg_manager::encore_install::{Frames, Reconstruction};
impl Manager {
    pub(super) fn encore_install_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = &self.c.encore_install else {
            self.c.confirm = None;
            return;
        };
        let choices = Rc::new(RefCell::new(plan.choices.clone()));
        let owner = cx.entity().downgrade();
        let language = self.c.tr.language.clone();
        let width = (window.viewport_size().width - px(60.)).min(window.rem_size() * 49.);
        let height = (window.viewport_size().height - window.rem_size() * 16.).max(px(150.));
        self.c.confirm = None;
        self.dialog_active = true;
        window.open_dialog(cx,move|dialog,_,cx|{
            let tr=i18n::Translator::new(&language);
            let mut body=div().id("encore-install-plan").v_flex().gap_3().max_h(height).overflow_y_scroll().text_sm();
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
                    if !choice.dlss_fg_available {row=row.child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr.t("检测到现有 Streamline 或组件冲突，新增 DLSS-G 暂不可用；保留原文件，可选择其他引擎。")));}
                } else {row=row.child(div().text_xs().child(tr.t("保留游戏原生帧生成")));}
                if choice.needs_reconstruction {
                    let mut buttons=div().h_flex().flex_wrap().gap_1();
                    for (n,sr,label) in [(0,Reconstruction::None,"不启用"),(1,Reconstruction::Dlss,"DLSS"),(2,Reconstruction::Xess,"XeSS")] {
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
                body=body.child(row);
            }
            body=body.child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child(tr.t("安装内容：Encore 与配套 NVIDIA / Intel 运行库；原生组件按实际路径更新并备份，仅安全适用时补充新增 DLSS-G 文件。新增功能为实验性，安装后重启游戏。")));
            let ready=choices.borrow().iter().all(|c|c.values().is_some());
            let accept=owner.clone();let cancel=owner.clone();let selections=choices.clone();
            dialog.title(crate::ui_focus::dialog_title(tr.t("首次安装 Encore：选择引擎").into(),cx)).width(width).close_button(false)
                .child(body)
                .footer(DialogFooter::new()
                    .child(Button::new("engines-cancel").small().label(tr.t("取消")).on_click(|_,w,cx|w.dispatch_action(Box::new(Cancel),cx)))
                    .child(Button::new("engines-install").small().primary().disabled(!ready).label(tr.t("安装并应用"))
                        .on_click(|_,w,cx|w.dispatch_action(Box::new(Confirm{secondary:false}),cx))))
                .on_ok(move|_,_,cx|{
                    if !selections.borrow().iter().all(|c|c.values().is_some()){return false;}
                    let _=accept.update(cx,|owner,cx|{owner.dialog_active=false;
                        if let Err(e)=owner.c.confirm_encore_install(selections.borrow().clone()){owner.c.warning=Some(e.to_string());}
                        cx.notify();});true
                })
                .on_cancel(move|_,_,cx|{let _=cancel.update(cx,|owner,cx|{owner.dialog_active=false;owner.c.cancel_patch();cx.notify();});true})
        });
    }
}
