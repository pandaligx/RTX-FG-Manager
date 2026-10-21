# Windows manager compatibility patch

Baseline: the published `gpui-fast-windows` **0.1.10** crate from crates.io
(`gpui-fast-windows-0.1.10.crate` SHA-256
`f034e131f83ff84b67f009cde41e0cbecb3be09d7ebba7c548ea78fe0fdee5b0`).
The crate's Apache-2.0 `LICENSE`, README and original manifest are retained.
The normalized standalone manifest pins its `gpui-fast` dependency to `=0.1.10`
and sets `publish = false`.

The manager previously applied equivalent compatibility changes to Zed commit
`1d217ee39d381ac101b7cf49d3d22451ac1093fe`. These changes are reapplied to the
published Fast backend without replacing its rendering, composition, device
recovery, scheduling or retained-layer implementations.
For manager 4.3.6 they were rebased from the published 0.1.2 crate onto 0.1.10,
retaining the newer rounded clipping, class names and foreign-message handling.
The unused upstream `HWND` import in `fast/composition.rs` is also removed.
The shared `fast/rounded_clip_fixtures.rs` test input is copied from
the matching published `gpui-fast` 0.1.10 crate (Apache-2.0). The Windows pixel
tests refer to that local copy instead of an absent sibling in the upstream
monorepo. It is only compiled with `test-support`, not in a manager release.
One interior 50% green sample accepts both 127 and 128 for Direct3D UNORM
rounding; its other channels, opacity and all clip-edge checks remain exact.

- `fast/startup.rs`: apply final window size while hidden, request a first frame
  after GPUI registers its frame callback, then defer native visibility until a
  successful draw has returned. Both regular and composed drawing use this path.
  A renderer frame skipped during device recovery does not expose the window.
  Native focus preference is preserved. Deferral avoids synchronous size/focus
  callbacks while the renderer borrow and GPUI draw coordinator are held.
  First-frame posts carry the platform validation token required by 0.1.10;
  an untagged post would now be discarded, leaving the initial window hidden.
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
