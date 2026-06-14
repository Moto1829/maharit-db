"""E2E スクリプト用の出力ヘルパー。

- ANSI カラー定数
- `Reporter`: pass/fail を集計し、終了コード相当を返す
- 互換のため、モジュールレベルの `check` / `section` / `summarize` も提供する
  （内部的にデフォルト Reporter インスタンスを使う）
"""

from __future__ import annotations


# ── ANSI カラー ──────────────────────────────────────────────────────────────

GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
CYAN = "\033[96m"
BOLD = "\033[1m"
RESET = "\033[0m"


class Reporter:
    """テスト結果の集計と表示を担当する。スクリプトごとに 1 インスタンス。"""

    def __init__(self) -> None:
        self.passed = 0
        self.failed = 0

    def check(self, name: str, condition: bool, detail: str = "") -> None:
        if condition:
            self.passed += 1
            print(f"  {GREEN}✓{RESET} {name}")
        else:
            self.failed += 1
            detail_str = f" — {detail}" if detail else ""
            print(f"  {RED}✗{RESET} {name}{detail_str}")

    def section(self, title: str) -> None:
        print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")

    def summarize(self) -> int:
        """結果サマリーを出力し、終了コードを返す（0 = 全 PASS, 1 = 1 件以上 FAIL）。"""
        total = self.passed + self.failed
        print(f"\n{BOLD}{'─' * 50}{RESET}")
        if self.failed == 0:
            print(f"{GREEN}{BOLD}✓ 全 {total} 件通過{RESET}")
            return 0
        print(f"{RED}{BOLD}✗ {self.failed} 件失敗 / {total} 件中{RESET}")
        return 1


# ── モジュールレベルのデフォルト Reporter ────────────────────────────────────
# 既存スクリプトとの互換維持用。新規スクリプトでは Reporter() を直接使う方が望ましい。

_default = Reporter()


def check(name: str, condition: bool, detail: str = "") -> None:
    _default.check(name, condition, detail)


def section(title: str) -> None:
    _default.section(title)


def summarize() -> int:
    return _default.summarize()


def stats() -> tuple[int, int]:
    """(passed, failed) を返す。"""
    return _default.passed, _default.failed
