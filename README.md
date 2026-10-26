# EpubReader

The EPUB reader plugin for [MimiReader](https://github.com/villatothesea/MimiReader) —
EPUB 2 and 3 e-books (DRM-free), read-only.

A plugin is a separate executable that speaks line-delimited JSON over stdio. The
protocol and the serve loop come from `mireader-reader-sdk`. A package is a folder
holding `reader.toml` and the executable, found and hot-swapped by
`mireader-reader-host`. Both crates live in the reader's repository, and this repo
expects it as a sibling folder:

```text
<workspace>\
  MimiReader\   the reader (crates\reader-sdk, crates\reader-host)
  EpubReader\   this repository
```

## Build and test

```powershell
cargo test -j 2      # unit tests and the host end to end
.\stage.ps1          # build and stage into ..\MimiReader\target\debug\readers\mireader.epub
.\stage.ps1 -Release -Out D:\Programs\MimiReader\readers
```

Licence: MIT (see `LICENSE`).
