# Windows manager compatibility patch

Baseline: the published `gpui-fast-windows` **0.1.2** crate from crates.io
(`gpui-fast-windows-0.1.2.crate` SHA-256
`3151bd462aecf475efb28f951573a66e655749a3af8d69a66a30e0e8c03b569a`).
The crate's Apache-2.0 `LICENSE`, README and original manifest are retained.
The normalized standalone manifest pins its `gpui-fast` dependency to `=0.1.2`;
only `publish = false` is changed locally.

The manager previously applied equivalent compatibility changes to Zed commit
`1d217ee39d381ac101b7cf49d3d22451ac1093fe`. These changes are reapplied to the
published Fast backend without replacing its rendering, composition, device
recovery, scheduling or retained-layer implementations.

- `fast/startup.rs`: apply final window size while hidden, request a first frame
  after GPUI registers its frame callback, then defer native visibility until a
  successful draw has returned. Both regular and composed drawing use this path.
  A renderer frame skipped during device recovery does not expose the window.
  Native focus preference is preserved. Deferral avoids synchronous size/focus
  callbacks while the renderer borrow and GPUI draw coordinator are held.
- `fast/compat.rs`: resolve `GetDpiForWindow` and `GetSystemMetricsForDpi`
  dynamically; retain monitor-DPI/system-metric fallbacks on earlier Windows 10.
- `fast/font_loader.rs` and `direct_write.rs`: preserve the earlier Windows 10
  DirectWrite path (`IDWriteFactory3`, `IDWriteFontSetBuilder`, custom
  `IDWriteFontFileLoader`/`IDWriteFontFileStream`). Loader and stream ownership
  keeps supplied font bytes alive. Font files are analyzed and their face
  references added individually instead of using the Windows 10 1703 loader.
  `directx_renderer.rs` obtains rendering parameters from `IDWriteFactory`.
- `fast/color_glyphs.rs`: use the modern `IDWriteFactory4` overload when the
  installed DirectWrite implements it, preserving the baseline format flags.
  Otherwise use the original COLR enumerator. Both variants call `MoveNext`
  before their first `GetCurrentRun`; errors are propagated. Two native-font
  regressions cover capability selection and the forced legacy path.
- The published baseline already uses `GetUserDefaultLocaleName` for its locale,
  but its Jump List code still imports ICU's `u_strlen`. `destination_list.rs`
  replaces that call with a bounded Rust scan of the UTF-16 description buffer,
  preserving the old manager's lack of an external ICU loader dependency.

The font-byte lifetime/range tests and hidden-window placement regression are
retained in the compatibility modules. Their execution on a development Windows
machine does not establish older Windows 10, physical mixed-DPI, GPU or game
compatibility. The application build's lockfile and release validation remain
required.
