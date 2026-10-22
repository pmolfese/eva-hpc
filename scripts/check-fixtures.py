"""Verify that the checked-in reference fixtures remain byte-for-byte unchanged."""
from pathlib import Path
import hashlib
import json

base = Path(__file__).resolve().parents[1] / 'crates/eva-mff/tests/fixtures'
for name, expected in json.loads((base / 'sha256.json').read_text()).items():
    actual = hashlib.sha256((base / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f'Fixture changed: {name}')
print('Fixture hashes verified')
