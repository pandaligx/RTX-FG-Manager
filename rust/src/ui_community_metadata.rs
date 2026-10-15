use super::*;
use rtx_fg_manager::community::Measurement;

pub struct MetadataFields {
    inputs: BTreeMap<&'static str, (String, Entity<InputState>)>,
}
impl MetadataFields {
    pub fn new(
        tr: &rtx_fg_manager::i18n::Translator,
        window: &mut Window,
        cx: &mut Context<CommunityView>,
    ) -> Self {
        Self {
            inputs: [
                ("model", "显卡型号"),
                ("driver", "驱动版本"),
                ("dlss", "DLSS 模式（可选）"),
                ("rt", "光追等级（可选）"),
                ("base", "原生 FPS（玩家填写，可选）"),
                ("output", "输出 FPS（玩家填写，可选）"),
                ("low", "1% Low FPS（可选）"),
                ("scene", "测试场景与方法（有测量时必填）"),
            ]
            .into_iter()
            .map(|(key, label)| {
                (
                    key,
                    (
                        tr.t(label),
                        cx.new(|cx| InputState::new(window, cx).placeholder(tr.t(label))),
                    ),
                )
            })
            .collect(),
        }
    }
    fn text(&self, key: &str, cx: &App) -> Option<String> {
        let value = self.inputs[key].1.read(cx).value().trim().to_owned();
        (!value.is_empty()).then_some(value)
    }
    pub fn hardware(&self, cx: &App) -> anyhow::Result<Hardware> {
        Ok(Hardware {
            gpu_model: self.text("model", cx),
            driver: self.text("driver", cx),
        })
    }
    pub fn measurement(&self, cx: &App) -> anyhow::Result<Option<Measurement>> {
        let number = |key: &str| -> anyhow::Result<Option<f64>> {
            self.text(key, cx)
                .map(|v| {
                    let n = v
                        .parse::<f64>()
                        .with_context(|| format!("{}需要填写数字", self.inputs[key].0))?;
                    anyhow::ensure!(
                        n.is_finite() && (0.0..=10000.0).contains(&n),
                        "FPS 数值无效"
                    );
                    Ok(n)
                })
                .transpose()
        };
        let (base_fps, output_fps, one_percent_low) =
            (number("base")?, number("output")?, number("low")?);
        if base_fps.is_none() && output_fps.is_none() && one_percent_low.is_none() {
            return Ok(None);
        }
        let scene = self
            .text("scene", cx)
            .context("请填写测试场景与方法；这些数据将标记为玩家提供")?;
        Ok(Some(Measurement {
            source: "player".into(),
            base_fps,
            output_fps,
            one_percent_low,
            scene,
        }))
    }
    pub fn quality(&self, cx: &App) -> (Option<String>, Option<String>) {
        (self.text("dlss", cx), self.text("rt", cx))
    }
    pub fn set_hardware(&self, h: &Hardware, window: &mut Window, cx: &mut Context<CommunityView>) {
        for (key, value) in [("model", h.gpu_model.clone()), ("driver", h.driver.clone())] {
            self.inputs[key].1.update(cx, |s, cx| {
                s.set_value(value.unwrap_or_default(), window, cx)
            });
        }
    }
    pub fn render(&self, tr: &rtx_fg_manager::i18n::Translator) -> AnyElement {
        let mut panel = div()
            .v_flex()
            .gap_2()
            .child(tr.t("只核对显卡型号和驱动版本；留空表示未知。"));
        for key in [
            "model", "driver", "dlss", "rt", "base", "output", "low", "scene",
        ] {
            let (label, input) = &self.inputs[key];
            panel = panel.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().w(px(210.)).text_sm().child(label.clone()))
                    .child(Input::new(input)),
            );
        }
        panel.into_any_element()
    }
}
