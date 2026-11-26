#!/usr/bin/env python3
"""Render the Agent Relay documentation site into an empty output directory; stdlib only."""
import argparse
from html import escape
import os
from pathlib import Path
import shutil
from urllib.parse import urlsplit

BASE_URL = os.environ.get("DOCS_BASE_URL", "https://relay.ai.swacktech.com").rstrip("/")
if (
    urlsplit(BASE_URL).scheme != "https"
    or not urlsplit(BASE_URL).hostname
    or urlsplit(BASE_URL).path
    or urlsplit(BASE_URL).query
    or urlsplit(BASE_URL).fragment
    or urlsplit(BASE_URL).username
    or urlsplit(BASE_URL).password
):
    raise SystemExit("DOCS_BASE_URL must be an HTTPS origin")

CNAME_HOST = urlsplit(BASE_URL).hostname
ROOT = Path(__file__).resolve().parent
PAGES = {
    "index": (
        "Agent Relay — Cross-agent messaging & live Relay Map UI",
        "Let Claude Code, Codex, Antigravity, opencode, and pi sessions message each other locally or over ssh, with an interactive Relay Map MCP App.",
    ),
    "relay-map": (
        "Relay Map UI tour — Agent Relay",
        "Visual tour of the Relay Map pew-pew MCP App: orbital traffic canvas, agent inspector, health alerts, spend analytics, session logs, and broadcast controls.",
    ),
    "install": (
        "Install and connect — Agent Relay",
        "Install Agent Relay in Antigravity, Claude Code, Claude Desktop, Codex, opencode, and pi, and configure multi-host ssh messaging.",
    ),
    "workflows": (
        "Workflows, routing, and Kubernetes — Agent Relay",
        "Cross-client message routing, five bundled skills, peer-framing safety model, and the optional standalone Rust/WASM Kubernetes hub.",
    ),
    "reference": (
        "Technical reference — Agent Relay",
        "Complete specification of the 7 relay MCP tools, relay.py CLI subcommands, per-client lifecycle hooks, and state layout.",
    ),
}

NAV_ITEMS = [
    ("index", "Overview"),
    ("relay-map", "Relay Map UI"),
    ("install", "Install"),
    ("workflows", "Workflows"),
    ("reference", "Reference"),
]


def render_shell(slug: str, title: str, description: str, body: str) -> str:
    nav_links = "".join(
        f'<a href="{page}.html"'
        + (' aria-current="page"' if page == slug else "")
        + f">{escape(label)}</a>"
        for page, label in NAV_ITEMS
    )
    canonical_suffix = "" if slug == "index" else f"{slug}.html"
    canonical_url = f"{BASE_URL}/{canonical_suffix}"
    return (
        f'<!doctype html>\n<html lang="en"><head><meta charset="utf-8">'
        f'<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>{escape(title)}</title>"
        f'<meta name="description" content="{escape(description, quote=True)}">'
        f'<meta name="theme-color" content="#0b1220">'
        f'<meta property="og:title" content="{escape(title, quote=True)}">'
        f'<meta property="og:description" content="{escape(description, quote=True)}">'
        f'<meta property="og:type" content="website">'
        f'<meta property="og:url" content="{escape(canonical_url, quote=True)}">'
        f'<link rel="canonical" href="{escape(canonical_url, quote=True)}">'
        f'<link rel="icon" href="assets/favicon.svg" type="image/svg+xml">'
        f'<link rel="stylesheet" href="assets/style.css">'
        f'<script src="assets/site.js" defer></script></head><body>'
        f'<a class="skip" href="#main">Skip to content</a>'
        f'<header><div class="header-inner">'
        f'<a class="brand" href="index.html"><span class="brand-mark" aria-hidden="true">◉</span>Agent Relay</a>'
        f'<nav aria-label="Main navigation">{nav_links}'
        f'<a href="https://ai.swacktech.com/plugins/agent-relay/">Marketplace ↗</a>'
        f'<a href="https://github.com/swack-tools/agent-relay-ai-plugin">GitHub ↗</a></nav>'
        f"</div></header>"
        f'<main id="main">{body}</main>'
        f'<footer><div><a class="brand" href="index.html">Agent Relay</a>'
        f"<p>Cross-client AI agent messaging &amp; Relay Map observability by SwackTech LLC.</p></div>"
        f'<p><a href="https://ai.swacktech.com/plugins/agent-relay/">Swack Tools Marketplace</a> · '
        f'<a href="https://github.com/swack-tools/agent-relay-ai-plugin">Source on GitHub</a> · '
        f'<a href="https://github.com/swack-tools/agent-relay-ai-plugin/releases/latest">Releases</a> · '
        f'<a href="https://github.com/swack-tools/agent-relay-ai-plugin/blob/main/LICENSE">GPL-3.0-only License</a><br>'
        f"Independent project by SwackTech LLC. Not endorsed by Anthropic, OpenAI, Google, or the opencode and pi projects.</p></footer>"
        f"</body></html>\n"
    )


def build(destination: Path) -> None:
    destination = destination.resolve()
    if destination.exists() and any(destination.iterdir()):
        raise SystemExit("Output directory must be empty; choose a fresh directory.")
    destination.mkdir(parents=True, exist_ok=True)
    for name, (title, description) in PAGES.items():
        body = (ROOT / "pages" / f"{name}.html").read_text(encoding="utf-8")
        (destination / f"{name}.html").write_text(
            render_shell(name, title, description, body), encoding="utf-8"
        )
    not_found_body = (
        '<p class="eyebrow">404 · Not found</p>'
        "<h1>Page not found</h1>"
        '<p class="lead">The requested documentation page does not exist on this site.</p>'
        '<div class="actions"><a class="button" href="index.html">Return to overview →</a>'
        '<a class="button secondary" href="relay-map.html">Explore Relay Map UI</a></div>'
    )
    (destination / "404.html").write_text(
        render_shell(
            "404",
            "Page not found — Agent Relay",
            "The requested Agent Relay documentation page could not be found.",
            not_found_body,
        ),
        encoding="utf-8",
    )
    shutil.copytree(ROOT / "assets", destination / "assets")
    (destination / "CNAME").write_text(f"{CNAME_HOST}\n", encoding="utf-8")
    (destination / ".nojekyll").touch()
    sitemap_urls = "".join(
        f"<url><loc>{BASE_URL}/{'' if slug == 'index' else slug + '.html'}</loc></url>"
        for slug in PAGES
    )
    (destination / "sitemap.xml").write_text(
        f'<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{sitemap_urls}</urlset>\n',
        encoding="utf-8",
    )
    print(f"Built {len(PAGES) + 1} pages in {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    build(parser.parse_args().output)
