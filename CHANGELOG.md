# Changelog

## [1.2.5](https://github.com/mecoren/EcoPaste/compare/v1.2.4...v1.2.5) (2026-09-15)

### ✨ Features

* make the clipboard hover preview fully interactive: scroll long content, drag-select text and copy just the selected fragment via the floating or header button, and preview single-image files in a lightbox

### 🐛 Bug Fixes

* clicking inside the interactive preview panel no longer dismisses the preview (the main window blur from focusing the overlay was closing it)

## [1.2.4](https://github.com/mecoren/EcoPaste/compare/v1.2.3...v1.2.4) (2026-09-15)

### ✨ Features

* navigate categories and source app with arrow keys ([83255d0](https://github.com/mecoren/EcoPaste/commit/83255d0))

## [1.2.3](https://github.com/mecoren/EcoPaste/compare/v1.2.2...v1.2.3) (2026-09-15)

### 🐛 Bug Fixes

* point update checks, download endpoint and release links to mecoren/EcoPaste

## [1.2.2](https://github.com/mecoren/EcoPaste/compare/v1.2.1...v1.2.2) (2026-09-15)

### 🐛 Bug Fixes

* pin clipboard timestamp to top-right until hover ([3084e8a](https://github.com/mecoren/EcoPaste/commit/3084e8a))

## [1.2.1](https://github.com/mecoren/EcoPaste/compare/v1.2.0...v1.2.1) (2026-09-15)

### ✨ Features

* custom preference window titlebar on Windows ([cc99305](https://github.com/mecoren/EcoPaste/commit/cc99305))

### 🐛 Bug Fixes

* bind dev server to IPv4 for tauri dev detection ([e77248f](https://github.com/mecoren/EcoPaste/commit/e77248f))
* correct NSAttributedString usage for macOS rich text conversion ([fac8c5d](https://github.com/mecoren/EcoPaste/commit/fac8c5d))
* replace memory metrics platform probes with sysinfo ([fb22dcf](https://github.com/mecoren/EcoPaste/commit/fb22dcf))

## [1.2.0](https://github.com/mecoren/EcoPaste/compare/v1.1.0...v1.2.0) (2026-09-14)

### ✨ Features

* add batch clipboard item operations (favorite, pin, note, copy, delete) ([3ed4b89](https://github.com/mecoren/EcoPaste/commit/3ed4b89))
* add database compaction and clear-on-exit history ([7c59855](https://github.com/mecoren/EcoPaste/commit/7c59855))
* add image preview lightbox with origin image ([ff3afae](https://github.com/mecoren/EcoPaste/commit/ff3afae))
* add list loading feedback, back-to-top, density presets and parallel quick actions ([361224a](https://github.com/mecoren/EcoPaste/commit/361224a))
* add memory observability and storage watermark ([f2ab1ab](https://github.com/mecoren/EcoPaste/commit/f2ab1ab))
* add multi-select batch delete and shift-number plain paste ([b9e4842](https://github.com/mecoren/EcoPaste/commit/b9e4842))
* add optional main window idle destroy ([67294c7](https://github.com/mecoren/EcoPaste/commit/67294c7))
* add rich text preview with sanitized sandbox iframe ([e70d1c7](https://github.com/mecoren/EcoPaste/commit/e70d1c7))
* filter clipboard items by source app ([daead85](https://github.com/mecoren/EcoPaste/commit/daead85))
* keep clipboard search ready for type-to-search like Ditto ([3718540](https://github.com/mecoren/EcoPaste/commit/3718540))
* support editing text clipboard item content ([a341ced](https://github.com/mecoren/EcoPaste/commit/a341ced))

### 🐛 Bug Fixes

* restore search focus on Backspace after arrow navigation ([eed9572](https://github.com/mecoren/EcoPaste/commit/eed9572))

### ⚡️ Performance

* batch backup merge imports ([7e2962c](https://github.com/mecoren/EcoPaste/commit/7e2962c))
* defer source app icon extraction off the watcher thread ([edfaa03](https://github.com/mecoren/EcoPaste/commit/edfaa03))
* dedupe plain-text search_text storage ([c784e87](https://github.com/mecoren/EcoPaste/commit/c784e87))
* incremental clipboard list updates and modifier-key link styling via CSS ([739b1f3](https://github.com/mecoren/EcoPaste/commit/739b1f3))
* mimalloc and preview downsampling ([65706ed](https://github.com/mecoren/EcoPaste/commit/65706ed))
* skip FTS rebuild on metadata-only updates ([0795cc8](https://github.com/mecoren/EcoPaste/commit/0795cc8))
* speed up clipboard hot path with sort indices and batched icon prefetch ([8ec2eeb](https://github.com/mecoren/EcoPaste/commit/8ec2eeb))
* take clipboard item by value in persist path ([a96928d](https://github.com/mecoren/EcoPaste/commit/a96928d))
* watcher hot path and count governance ([ab91fee](https://github.com/mecoren/EcoPaste/commit/ab91fee))

## [1.1.0](https://github.com/EcoPasteHub/EcoPaste/compare/v1.0.0...v1.1.0) (2026-07-22)

### ✨ Features

* add image save context menu ([#1353](https://github.com/EcoPasteHub/EcoPaste/issues/1353)) ([746cc05](https://github.com/EcoPasteHub/EcoPaste/commit/746cc0558d656ddb694c47c81a46549b89139c79))
* add nightly update channel support ([#1350](https://github.com/EcoPasteHub/EcoPaste/issues/1350)) ([9544725](https://github.com/EcoPasteHub/EcoPaste/commit/95447258f9e443d5285d46b775c7279d3d14fddc))

### 🐛 Bug Fixes

* correct timezone offset for legacy data import ([#1338](https://github.com/EcoPasteHub/EcoPaste/issues/1338)) ([d6ea9ed](https://github.com/EcoPasteHub/EcoPaste/commit/d6ea9ed4d1ce6af99673a53a3c1ed78068f83d53))
* deduplicate Windows autostart entries ([#1355](https://github.com/EcoPasteHub/EcoPaste/issues/1355)) ([0d87458](https://github.com/EcoPasteHub/EcoPaste/commit/0d87458b70323da27456cfaa16b51dcde9ea220e))
* prevent app freezes that may cause history loss ([#1342](https://github.com/EcoPasteHub/EcoPaste/issues/1342)) ([7a75196](https://github.com/EcoPasteHub/EcoPaste/commit/7a75196e90f4a4abfb49d9f84c591d80c735a468))
* quote windows autostart paths ([#1369](https://github.com/EcoPasteHub/EcoPaste/issues/1369)) ([d9d718e](https://github.com/EcoPasteHub/EcoPaste/commit/d9d718e456173e62f112ebd96609c049a331a1e8))
* retry Windows clipboard reads for screenshot capture ([#1367](https://github.com/EcoPasteHub/EcoPaste/issues/1367)) ([e6f7e80](https://github.com/EcoPasteHub/EcoPaste/commit/e6f7e80e83ae18681a976929d710d99119168866))
* show logo for clipboard items without source app ([#1370](https://github.com/EcoPasteHub/EcoPaste/issues/1370)) ([ad9eee0](https://github.com/EcoPasteHub/EcoPaste/commit/ad9eee0f1b7e3abe6546d4b3ac0fc8d4899bd5cb))
* skip invalid legacy import rows ([#1326](https://github.com/EcoPasteHub/EcoPaste/issues/1326)) ([c0f2d58](https://github.com/EcoPasteHub/EcoPaste/commit/c0f2d587677ab51f9c707f1e50fda6a83ce78a3e))
* update beta preference copy ([#1329](https://github.com/EcoPasteHub/EcoPaste/issues/1329)) ([185bb25](https://github.com/EcoPasteHub/EcoPaste/commit/185bb2520e8ce90da8246c0c73b1ce87e54403e5))

## [1.0.0](https://github.com/EcoPasteHub/EcoPaste/compare/v0.6.0-beta.3...v1.0.0) (2026-07-03)

This is a fully refactored version of EcoPaste. The app is lighter, faster, and more stable overall.

### ✨ Features

- EcoPaste can now open on Windows without taking focus from other windows.
- Added a Windows Win+V toggle, so EcoPaste can replace the system clipboard panel.
- Added first-run onboarding for permissions, shortcuts, ignored apps, and legacy data import.
- Added source app detection and ignored apps, so you can control which apps are excluded from clipboard history.
- Added capture preferences for saved content types, size limits, and priority.
- Added sensitive content protection to automatically skip high-risk content such as private keys, tokens, AWS keys, and JWTs.
- Added full content preview for text, image, and file records.
- Added drag-out support for dragging text, rich text, images, and file records into external apps.
- Added deletion protection for favorites and pinned items.
- Added preference search and preference reset.
- Backup import and export now support encrypted backups and merge import.
- Added a Windows administrator launch setting.
- Moved software updates into a dedicated window for checking, downloading, and installing new versions.
- More new features are ready for you to download, try, and explore.

### 🐛 Bug Fixes

- Fixed autostart not working when EcoPaste runs as administrator on Windows.

### ⚡️ Performance

- Clipboard monitoring, search, list rendering, and content preview are faster and more stable.
- App startup, everyday usage, and background resource usage have been further optimized.
- Image thumbnails, app icons, and file icons now load more efficiently.
- Added lightweight mode to reduce background refresh work and memory usage after the window is hidden.

### ⚠️ Upgrade Notice

- This refactored version only supports macOS and Windows.
- This version supports migrating history data from the legacy app during first-run onboarding.
