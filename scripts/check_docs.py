#!/usr/bin/env python3
"""Check navigation coverage, README links and the built documentation site."""

from collections import Counter
from html.parser import HTMLParser
from pathlib import Path
import sys
import tomllib
from urllib.parse import unquote, urljoin, urlsplit

import markdown

ROOT = Path(__file__).resolve().parents[1]


class Links(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.links = []
        self.ids = set()
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        for name in ("href", "src"):
            if attrs.get(name):
                self.links.append(attrs[name])


def nav_paths(nav):
    for item in nav:
        if isinstance(item, str):
            yield item
        elif isinstance(item, dict):
            for value in item.values():
                if isinstance(value, list):
                    yield from nav_paths(value)
                else:
                    yield value


def main():
    config = tomllib.loads((ROOT / "zensical.toml").read_text())["project"]
    docs = ROOT / config["docs_dir"]
    site = ROOT / config["site_dir"]
    errors = []
    pages = {str(path.relative_to(docs)) for path in docs.rglob("*.md")}
    listed = Counter(path for path in nav_paths(config["nav"])
                     if not urlsplit(path).scheme)
    for page in sorted(pages | listed.keys()):
        if page not in pages or listed[page] != 1:
            errors.append(f"navigation: {page} must exist and be listed exactly once")

    readme = Links(markdown.markdown((ROOT / "README.md").read_text()))
    for link in readme.links:
        parsed = urlsplit(link)
        if not parsed.scheme and not parsed.netloc and parsed.path:
            if not (ROOT / unquote(parsed.path)).exists():
                errors.append(f"README.md: missing {link}")

    html_pages = {path: Links(path.read_text()) for path in site.rglob("*.html")}
    if site / "index.html" not in html_pages:
        errors.append("site/index.html missing: run zensical build --strict first")
    base = config["site_url"]
    base_url = urlsplit(base)
    for source, html in html_pages.items():
        relative = source.relative_to(site).as_posix()
        page_url = urljoin(base, relative.removesuffix("index.html"))
        for link in html.links:
            target_url = urlsplit(urljoin(page_url, link))
            if (target_url.scheme, target_url.netloc) != (base_url.scheme, base_url.netloc):
                continue
            path = unquote(target_url.path)
            if not path.startswith(base_url.path):
                errors.append(f"{relative}: link escapes project subpath: {link}")
                continue
            target = site / path[len(base_url.path):]
            if target.is_dir():
                target /= "index.html"
            if not target.is_file():
                errors.append(f"{relative}: missing {link}")
            elif target_url.fragment and target in html_pages:
                if unquote(target_url.fragment) not in html_pages[target].ids:
                    errors.append(f"{relative}: missing anchor in {link}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Documentation checks passed: {len(pages)} pages, README and site links")
    return 0


if __name__ == "__main__":
    sys.exit(main())
