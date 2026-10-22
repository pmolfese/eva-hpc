"""Build/run a separate Cargo workspace consuming eva-mff through its public API."""
from pathlib import Path
import json
import os
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='eva-mff-consumer-') as tmp:
    p = Path(tmp)
    (p / 'src').mkdir()
    (p / 'Cargo.toml').write_text(
        '[package]\nname = "mff-consumer-check"\nversion = "0.0.0"\nedition = "2021"\n'
        '[workspace]\n[dependencies]\neva-mff = { path = '
        + json.dumps(str(root / 'crates/eva-mff')) + ' }\n')
    (p / 'src/main.rs').write_text('''use eva_mff::{Recording, SampleValues};
fn main() {
    let r = Recording::open(std::env::args_os().nth(1).unwrap()).unwrap();
    assert_eq!(r.metadata().sample_rate, 250);
    assert_eq!(r.read_window(&[0], 0..8, SampleValues::Raw).unwrap().values[0].len(), 8);
}
''')
    env = dict(os.environ, CARGO_TARGET_DIR=str(root / 'target/consumer-check'))
    subprocess.run(['cargo', 'run', '--offline', '--manifest-path', str(p / 'Cargo.toml'), '--',
                    str(root / 'crates/eva-mff/tests/fixtures/example_3.mff')], check=True, env=env)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--offline', '--format-version', '1',
                                                 '--manifest-path', str(p / 'Cargo.toml')], env=env))
    names = {package['name'] for package in metadata['packages']}
    assert not names.intersection({'egui', 'wgpu', 'eva-viewer'}), names
print('External consumer passed without graphics dependencies')
