"""mkdocs フック: 日本語版より古い英語ページの冒頭に注意書きを入れる。

判定は scripts/check_docs_i18n.py と同じ（source_commit 以降に日本語だけを変えた
コミットがあれば古い）。git の履歴が無い（浅いクローンなど）ときは何もしない。
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "check_docs_i18n.py"
_spec = importlib.util.spec_from_file_location("check_docs_i18n", _SCRIPT)
_check = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = _check  # dataclass の型解決に必要
_spec.loader.exec_module(_check)

NOTICE = (
    '!!! warning "This translation may be outdated"\n'
    "    The Japanese original of this page has changed since it was translated. "
    "See the Japanese version for the latest content.\n\n"
)

_stale: set[str] = set()


def on_pre_build(config, **kwargs):
    global _stale
    _stale = _check.stale_en_paths()


def on_page_markdown(markdown, page, config, files, **kwargs):
    if page.file.src_uri not in _stale or "This translation may be outdated" in markdown:
        return markdown
    return NOTICE + markdown
