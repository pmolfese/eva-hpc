# Third-party notices

The root [LICENSE](LICENSE) contains EVA's Government-work notice. It does not
waive rights or obligations attached to third-party software or data. This file
identifies the material directly present in this source tree and the dependency
manifests; it is not a complete generated license inventory for compiled binaries.

## MFF reference recordings

- Files: `crates/eva-mff/tests/fixtures/example_3.mff/` and `example_4.mff/`.
- Upstream: [BEL-Public/mffpy](https://github.com/BEL-Public/mffpy), `examples/`.
- Attribution: Copyright 2019 Brain Electrophysiology Laboratory Company LLC.
- License: Apache License, Version 2.0.
- Full retained notice/license:
  [LICENSE-mffpy.txt](crates/eva-mff/tests/fixtures/LICENSE-mffpy.txt).
- Copy provenance and integrity checks:
  [fixture README](crates/eva-mff/tests/fixtures/README.md) and
  [SHA-256 manifest](crates/eva-mff/tests/fixtures/sha256.json).

These fixtures were copied unmodified from the Swift EVA fixture collection.
The supplied license text names additional upstream examples; only the two
recording directories listed above are included here. Their license file is
retained verbatim.

## Rust dependencies

Third-party crates are declared in the workspace's `crates/*/Cargo.toml` manifests;
`Cargo.lock` records the resolved versions, including transitive dependencies.
Principal dependencies include egui/eframe, wgpu, rfd, RustFFT, roxmltree, serde,
serde_json, time, and tempfile. They retain their upstream licenses and notices.
They are not relicensed by the Government-work notice.

Dependency metadata can be inspected with `cargo metadata --format-version 1
--locked`. The original package distributions include their license files.
A binary distribution must retain the notices required by the dependencies
actually included in that build, including target-specific and transitive crates.
This source repository does not bundle a complete set of those license texts.

## Relationship to macOS EVA

The Rust reader and algorithms were developed using the macOS EVA implementation
and its documented behavior as references. Reader-reference provenance is recorded
in the fixture README. Comments identify the MRI engines as ports of EVA's Allen
AAS and clean-room FASTR-family mechanics. Method names and toolbox-convention
presets do not mean that those toolboxes are distributed or linked here.

The Swift application's third-party inventory covers a different source tree and
must not be treated as an inventory of this Rust application. When introducing
additional source, data, or dependencies, retain their original attribution and
update this file for material redistributed directly in the repository.
