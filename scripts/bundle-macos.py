"""Wrap an already-built viewer in a local, unsigned macOS .app for testing."""
from pathlib import Path
import argparse
import plistlib
import shutil

parser = argparse.ArgumentParser()
parser.add_argument('--release', action='store_true')
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
profile = 'release' if args.release else 'debug'
binary = root / 'target' / profile / 'eva-viewer'
if not binary.is_file():
    raise SystemExit(f'Build first: cargo build -p eva-viewer' + (' --release' if args.release else ''))
app = root / 'target' / 'EVA Viewer.app'
(app / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
shutil.copy2(binary, app / 'Contents/MacOS/eva-viewer')
cli = root / 'target' / profile / 'eva'
if cli.is_file():
    shutil.copy2(cli, app / 'Contents/MacOS/eva')
with (app / 'Contents/Info.plist').open('wb') as f:
    plistlib.dump({'CFBundleExecutable': 'eva-viewer', 'CFBundleIdentifier': 'org.eva-hpc.viewer',
                  'CFBundleName': 'EVA Viewer', 'CFBundleDisplayName': 'EVA Viewer',
                  'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1',
                  'CFBundleShortVersionString': '0.1.0', 'NSHighResolutionCapable': True,
                  'NSPrincipalClass': 'NSApplication'}, f)
print(app)
