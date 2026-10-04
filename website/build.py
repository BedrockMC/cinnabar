#!/usr/bin/env python3
"""Build the static download page from Cinnabar's canonical artwork and release manifest."""
import importlib.util
import json
from pathlib import Path
import random
import shutil
import xml.etree.ElementTree as ET
from urllib.parse import quote

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("release_downloads", ROOT / "packaging/release-downloads.py")
downloads = importlib.util.module_from_spec(spec)
spec.loader.exec_module(downloads)


def build(output):
    output.mkdir(parents=True, exist_ok=True)
    icon = (ROOT / "packaging/icons/cinnabar.svg").read_text()
    red = ET.fromstring(icon).find("{http://www.w3.org/2000/svg}rect").attrib["fill"]
    config = downloads.download_config()
    page = (HERE / "index.html.in").read_text()
    page = page.replace("@@RED@@", red).replace("@@FAVICON@@", "data:image/svg+xml," + quote(icon, safe=""))
    page = page.replace("@@REPOSITORY_URL@@", "https://github.com/" + config["repository"])
    (output / "index.html").write_text(page)
    (output / "downloads.js").write_text("window.CINNABAR_DOWNLOADS = " + json.dumps(config) + ";\n")
    (output / config["install_script"]).write_text(downloads.render_installer())
    shutil.copyfile(HERE / "app.js", output / "app.js")
    shutil.copyfile(ROOT / "assets/branding/title.png", output / "title.png")
    rng = random.Random(27)
    colors = ["#1c221d", "#202620", "#242a23", "#262d25", "#1f251f"]
    pixels = "".join(f'<rect x="{x}" y="{y}" width="16" height="16" fill="{rng.choice(colors)}"/>'
                     for y in range(0, 256, 16) for x in range(0, 256, 16))
    (output / "texture.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">' + pixels + "</svg>")


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / ".local/website")
    build(parser.parse_args().output)
