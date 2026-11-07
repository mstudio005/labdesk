#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Syed Mutaib
"""Build the extension for each browser.

  python3 build.py        makes  dist/chrome/   (Chrome, Brave, Edge: "Load unpacked")
                                 dist/firefox/  (Firefox: "Load Temporary Add-on", or upload to addons.mozilla.org)
                                 dist/labdesk-bridge-chrome.zip, dist/labdesk-bridge-edge.zip (same files, for the Edge Add-ons store)
                                 dist/labdesk-bridge-firefox.zip
Only the browser-specific parts of manifest.json differ.
"""
import json, os, shutil, zipfile

here = os.path.dirname(os.path.abspath(__file__))
dist = os.path.join(here, "dist")
base = json.load(open(os.path.join(here, "manifest.base.json"), encoding="utf-8"))
FILES = ["src", "icons"]


def make(kind):
    m = json.loads(json.dumps(base))
    m["manifest_version"] = 3
    if kind == "chrome":
        m["background"] = {"service_worker": "src/background.js", "type": "module"}
        m["minimum_chrome_version"] = "110"
    else:
        # Firefox runs the background as a page instead of a service worker.
        m["background"] = {"scripts": ["src/background.js"], "type": "module"}
        m["browser_specific_settings"] = {"gecko": {"id": "labdesk-bridge@mstudio005.github.io", "strict_min_version": "128.0",
                                                    "data_collection_permissions": {"required": ["none"]}}}
    out = os.path.join(dist, kind)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    for f in FILES:
        shutil.copytree(os.path.join(here, f), os.path.join(out, f))
    json.dump(m, open(os.path.join(out, "manifest.json"), "w", encoding="utf-8"), indent=2)
    return out


def zip_dir(folder, target):
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as z:
        for root, _, files in os.walk(folder):
            for f in sorted(files):
                full = os.path.join(root, f)
                z.write(full, os.path.relpath(full, folder).replace(os.sep, "/"))


os.makedirs(dist, exist_ok=True)
c, f = make("chrome"), make("firefox")
zip_dir(c, os.path.join(dist, "labdesk-bridge-chrome.zip"))
zip_dir(c, os.path.join(dist, "labdesk-bridge-edge.zip"))
zip_dir(f, os.path.join(dist, "labdesk-bridge-firefox.zip"))
print("built", os.listdir(dist))
