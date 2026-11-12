// Modified by Longbridge for gpui-fast.
//! Both debug runtime compilation and release FXC compilation consume the
//! same generated, self-contained shader. Upstream HLSL stays unchanged.

use std::path::{Path, PathBuf};

pub fn generated_path(out_dir: impl AsRef<Path>) -> PathBuf {
    out_dir.as_ref().join("rounded_clip_shaders.hlsl")
}

pub fn stage() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_path = root.join("src/shaders.hlsl");
    let alpha_path = root.join("src/alpha_correction.hlsl");
    println!("cargo:rerun-if-changed={}", shader_path.display());
    println!("cargo:rerun-if-changed={}", alpha_path.display());
    let source = source(
        &std::fs::read_to_string(shader_path).unwrap(),
        &std::fs::read_to_string(alpha_path).unwrap(),
    );
    std::fs::write(generated_path(std::env::var("OUT_DIR").unwrap()), source).unwrap();
}

pub fn source(original: &str, alpha: &str) -> String {
    let mut source = original.replace("#include \"alpha_correction.hlsl\"", alpha);
    for name in [
        "quad_fragment",
        "shadow_fragment",
        "underline_fragment",
        "monochrome_sprite_fragment",
        "polychrome_sprite_fragment",
        "path_sprite_fragment",
        "subpixel_sprite_fragment",
    ] {
        let start = source
            .find(&format!("{name}("))
            .expect("upstream HLSL fragment entry point");
        let body_start = start + source[start..].find('{').unwrap();
        let mut depth = 1;
        let mut body_end = body_start + 1;
        for (offset, ch) in source[body_start + 1..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                body_end = body_start + 1 + offset;
                break;
            }
        }
        let body = &source[body_start + 1..body_end];
        let body = if name == "subpixel_sprite_fragment" {
            assert!(body.contains("return output;"));
            body.replace(
                "return output;",
                "output.alpha *= fast_clip_coverage(input.position.xy); return output;",
            )
        } else {
            let mut result = String::new();
            let mut rest = body;
            while let Some(start) = rest.find("return ") {
                let end = start + rest[start..].find(';').expect("fragment return semicolon");
                result.push_str(&rest[..start]);
                result.push_str(&format!(
                    "return fast_clip_color({}, input.position.xy, {});",
                    &rest[start + 7..end],
                    name == "path_sprite_fragment"
                ));
                rest = &rest[end + 1..];
            }
            result.push_str(rest);
            result
        };
        source.replace_range(body_start + 1..body_end, &body);
    }
    format!(
        r#"
struct FastRoundedClip {{
  float4 bounds;
  float4 horizontal;
  float4 vertical;
}};
float fast_rounded_clip_coverage(float2 position, FastRoundedClip shape) {{
  float2 extent = shape.bounds.zw - shape.bounds.xy;
  if (extent.x <= 0 || extent.y <= 0) return 0;
  float2 local = position - shape.bounds.xy;
  float distance = max(max(-local.x, -local.y), max(local.x - extent.x, local.y - extent.y));
  [unroll] for (uint corner = 0; corner < 4; ++corner) {{
    float2 radius = float2(shape.horizontal[corner], shape.vertical[corner]);
    if (radius.x <= 0 || radius.y <= 0) continue;
    bool right = corner == 1 || corner == 2;
    bool bottom = corner >= 2;
    float2 inward = float2(right ? extent.x - local.x : local.x, bottom ? extent.y - local.y : local.y);
    if (inward.x < radius.x && inward.y < radius.y) {{
      float2 q = (inward - radius) / radius;
      float length_q = length(q);
      float gradient = length(q / radius) / max(length_q, 0.000001f);
      distance = max(distance, (length_q - 1) / max(gradient, 0.000001f));
    }}
  }}
  return clamp(0.5f - distance, 0.0f, 1.0f);
}}
Texture2D<float4> fast_clip_shapes : register(t15);
cbuffer FastClipParams : register(b2) {{ uint fast_clip_enabled; uint3 fast_clip_pad; }};
float fast_clip_coverage(float2 position) {{
    if (fast_clip_enabled == 0) return 1.0;
    uint width = 0, count = 0;
    fast_clip_shapes.GetDimensions(width, count);
    float coverage = 1.0;
    [loop] for (uint index = 0; index < count; ++index) {{
        FastRoundedClip shape;
        shape.bounds = fast_clip_shapes.Load(int3(0, index, 0));
        shape.horizontal = fast_clip_shapes.Load(int3(1, index, 0));
        shape.vertical = fast_clip_shapes.Load(int3(2, index, 0));
        coverage = min(coverage, fast_rounded_clip_coverage(position, shape));
    }}
    return coverage;
}}
float4 fast_clip_color(float4 color, float2 position, bool premultiplied) {{
    float coverage = fast_clip_coverage(position);
    if (premultiplied) color.rgb *= coverage;
    color.a *= coverage;
    return color;
}}
{source}
"#
    )
}
