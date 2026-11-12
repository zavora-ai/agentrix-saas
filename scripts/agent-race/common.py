"""Shared pieces of the agent race (R9.3): the model client, usage and cost, and run records.

Both customer agents are external clients of the sites, not the platform: they call the Claude
API directly with a key loaded from the repo's `.env` into this process only. The key is never
printed, logged or written to a result.
"""

from __future__ import annotations

import os
import pathlib
import time
from dataclasses import dataclass, field

import anthropic

REPO = pathlib.Path(__file__).resolve().parents[2]

# Claude Sonnet 5.5, first-party API rates per million tokens (claude-api skill, 2026-10-06).
MODEL = os.environ.get("RACE_MODEL", "claude-sonnet-5-5")
PRICE_IN = 2.00
PRICE_OUT = 10.00
PRICE_CACHE_WRITE = 2.50  # 1.25 x input, 5-minute cache
PRICE_CACHE_READ = 0.20


def load_key() -> None:
    """Puts ANTHROPIC_API_KEY from the repo's .env into this process's environment, unless it
    is already set. Reads only that one line; never prints it."""
    if os.environ.get("ANTHROPIC_API_KEY"):
        return
    env = REPO / ".env"
    if not env.exists():
        return
    for line in env.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("export "):
            line = line[len("export "):]
        if line.startswith("ANTHROPIC_API_KEY="):
            value = line.split("=", 1)[1].strip().strip('"').strip("'")
            if value:
                os.environ["ANTHROPIC_API_KEY"] = value
            return


def client() -> anthropic.Anthropic:
    load_key()
    return anthropic.Anthropic(max_retries=4, timeout=120)


@dataclass
class Usage:
    """Tokens and cost across one agent's model calls."""

    input: int = 0
    output: int = 0
    cache_write: int = 0
    cache_read: int = 0
    calls: int = 0

    def add(self, u) -> None:
        self.calls += 1
        self.input += getattr(u, "input_tokens", 0) or 0
        self.output += getattr(u, "output_tokens", 0) or 0
        self.cache_write += getattr(u, "cache_creation_input_tokens", 0) or 0
        self.cache_read += getattr(u, "cache_read_input_tokens", 0) or 0

    @property
    def cost(self) -> float:
        return (
            self.input * PRICE_IN
            + self.output * PRICE_OUT
            + self.cache_write * PRICE_CACHE_WRITE
            + self.cache_read * PRICE_CACHE_READ
        ) / 1_000_000

    def as_dict(self) -> dict:
        return {
            "model_calls": self.calls,
            "input_tokens": self.input,
            "output_tokens": self.output,
            "cache_write_tokens": self.cache_write,
            "cache_read_tokens": self.cache_read,
            "cost_usd": round(self.cost, 4),
        }


@dataclass
class Run:
    """One side of the race."""

    side: str  # "old" or "new"
    mode: str  # "order" or "question"
    target: str
    task: str
    started: float = field(default_factory=time.monotonic)
    wall_s: float = 0.0
    steps: int = 0
    outcome: str = "running"  # submitted | held | answered | gave_up | error | step_limit
    summary: str = ""
    final: dict = field(default_factory=dict)  # screenshot path or the door's final response
    log: list = field(default_factory=list)
    usage: Usage = field(default_factory=Usage)
    warnings: list = field(default_factory=list)

    def note(self, kind: str, **data) -> None:
        entry = {"t": round(time.monotonic() - self.started, 2), "kind": kind, **data}
        self.log.append(entry)
        brief = data.get("text") or data.get("action") or data.get("detail") or ""
        print(f"[{self.side}] {entry['t']:6.1f}s {kind}: {str(brief)[:140]}", flush=True)

    def finish(self, outcome: str, summary: str) -> None:
        self.outcome = outcome
        self.summary = summary
        self.wall_s = round(time.monotonic() - self.started, 2)

    def as_dict(self) -> dict:
        return {
            "side": self.side,
            "mode": self.mode,
            "target": self.target,
            "task": self.task,
            "model": MODEL,
            "wall_s": self.wall_s,
            "steps": self.steps,
            "outcome": self.outcome,
            "summary": self.summary,
            "usage": self.usage.as_dict(),
            "final": self.final,
            "warnings": self.warnings,
            "log": self.log,
        }


def create(cl: anthropic.Anthropic, run: Run, **kwargs):
    """One model call with the race's defaults: Claude Sonnet 5.5, adaptive thinking at the given
    effort, automatic prompt caching, and server-side refusal fallbacks. Usage is added to the
    run."""
    effort = kwargs.pop("effort", "medium")
    response = cl.beta.messages.create(
        model=MODEL,
        max_tokens=kwargs.pop("max_tokens", 8000),
        thinking={"type": "adaptive"},
        output_config={"effort": effort},
        cache_control={"type": "ephemeral"},
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        **kwargs,
    )
    run.usage.add(response.usage)
    if response.stop_reason == "refusal":
        details = getattr(response, "stop_details", None)
        run.note("refusal", detail=str(getattr(details, "category", "")))
    return response


def text_of(response) -> str:
    return " ".join(b.text for b in response.content if getattr(b, "type", "") == "text").strip()
