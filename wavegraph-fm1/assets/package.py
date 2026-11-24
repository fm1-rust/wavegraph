"""Run the packager shipped with WaveGraph's locked fm1-assets dependency."""

import json
from pathlib import Path
import runpy
import subprocess


def packager():
    manifest = Path(__file__).resolve().parents[1] / "Cargo.toml"
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version=1", "--manifest-path", str(manifest),
    ]))
    manifests = {p["id"]: Path(p["manifest_path"]) for p in metadata["packages"]}
    app = next(n for n in metadata["resolve"]["nodes"] if manifests[n["id"]] == manifest)
    assets = next(dep for dep in app["deps"] if dep["name"] == "fm1_assets")
    return manifests[assets["pkg"]].parent / "tools" / "package.py"


if __name__ == "__main__":
    runpy.run_path(str(packager()), run_name="__main__")
