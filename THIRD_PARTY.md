# Document Rendering

The PDF reader uses [Hayro](https://github.com/LaurenzV/hayro), a pure Rust
PDF parser/interpreter/rasterizer, under MIT OR Apache-2.0. Its embedded fonts
and character maps have their own notices in the Hayro dependency packages.

EPUB/FB2 text uses roxmltree (MIT) and cosmic-text / Swash (MIT OR Apache-2.0
and Apache-2.0 OR MIT respectively). XML decoding uses encoding_rs
(Apache-2.0 OR MIT OR BSD-3-Clause). CBZ uses zip (MIT) and image
(MIT OR Apache-2.0), with only Rust image codecs enabled.

MuPDF and its AGPL bindings are no longer used or linked by eNote.
Dependency licenses and notices are included in their Cargo source packages;
redistribution must comply with those licenses.
