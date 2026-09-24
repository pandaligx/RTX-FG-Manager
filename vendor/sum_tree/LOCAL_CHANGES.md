# Local dependency patch

Source: Zed commit 1d217ee39d381ac101b7cf49d3d22451ac1093fe, crates/sum_tree (Apache-2.0).

Removed seven optional `instrument(skip_all)` attributes and their ztracing imports, plus test-only zlog initialization. The original non-ztracing build macro returns its input unchanged. Tree/cursor algorithms are unchanged. This removes the GPL-only ztracing/zlog dependencies from the Windows manager build without copying their implementation. Package dependencies are made standalone; preserve Apache-2.0 notice.
