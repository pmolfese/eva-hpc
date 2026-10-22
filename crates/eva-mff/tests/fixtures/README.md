# Fixture provenance

`example_3.mff` and `example_4.mff` were copied unmodified from EVA's fixture
collection at commit `2c941d08e12f9baafde213d711570bfde2508139`. EVA attributes these
recordings to https://github.com/BEL-Public/mffpy, `examples/`, under Apache-2.0.
The complete supplied attribution and license are in `LICENSE-mffpy.txt`.
`sha256.json` records every copied regular file for verification.

- example_3: continuous 257-channel EEG at 250 Hz, 224 samples, additional PNS stream.
- example_4: categorized/segmented EEG with three events, used to test timing and
  conservative unsupported-timeline behavior.

Synthetic fixtures in `tests/reader.rs` are generated in temporary directories;
no participant data is used in those tests. Selected raw bit-pattern expectations
for example_3 were independently extracted with Python's `struct.unpack_from`
using each channel offset and the block header length, rather than the Rust reader.

The parser was informed by EVA's `EVACore/IO/MFFReader.swift` at the same commit.
That reference identifies its implementation as a work of the United States
Government, with international copyrights potentially applicable. The repository government-work notice is in the root `LICENSE`; the fixture
files retain their separate Apache-2.0 terms. The crate is not yet published to
crates.io.
