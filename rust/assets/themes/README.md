# Bundled GPUI Kit themes

The 21 JSON files in this directory are copied without modification from
[GPUI Kit](https://github.com/longbridge/gpui-kit/tree/4921e5b69834e45592c9772c57f39a29ada7de2f/themes),
commit `4921e5b69834e45592c9772c57f39a29ada7de2f`.

They contain 36 named variants: 11 light and 25 dark. Together with the Kit's
built-in Default Light and Default Dark, the manager offers 38 color variants.
The manager stores the light and dark selections independently. It does not
invent a light variant for a dark-only theme or a dark variant for a light-only
theme. The manager retains its Microsoft YaHei UI, 14 px interface typography.

The original theme-set names, authors and source URLs remain in each JSON file.
The upstream project's Apache 2.0 license and copyright notice are reproduced in
[`LICENSE-APACHE`](LICENSE-APACHE). The JSON files have not been modified; selection,
fallback and interface typography are implemented separately in `ui_themes.rs`.

Copyright 2024 - 2026 Longbridge <https://longbridge.com>

These assets are embedded at compile time. No theme download, working-directory
lookup or writable external theme folder is required at runtime.
