"""Tests for scripts/copy-lint. Run: python3 scripts/tests/test_copy_lint.py"""

import importlib.machinery
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True  # no scripts/__pycache__ left in the worktree

HERE = Path(__file__).resolve().parent
LINT = HERE.parent / "copy-lint"
_loader = importlib.machinery.SourceFileLoader("copy_lint", str(LINT))
_spec = importlib.util.spec_from_loader("copy_lint", _loader)
cl = importlib.util.module_from_spec(_spec)
sys.modules["copy_lint"] = cl
_loader.exec_module(cl)


def rules(src: str, name: str = "t.html", visitor: bool = False) -> list[str]:
    """The rules that fire on a snippet, in order."""
    path = Path(name)
    out = []
    for seg in cl.segments_for(path, src):
        out += [f[1] for f in cl.check(seg.text, seg.context, visitor)]
    return out


class Extraction(unittest.TestCase):
    def test_only_what_people_read(self):
        src = """<!doctype html>
<script>if (a !== b) alert("Oops!")</script>
<style>.x{color:red}</style>
{# a comment that says seamless #}
<!-- leverage this -->
<code>x != y!</code>
<p class="lead">{{ seamless_var }} Paste your website address</p>"""
        self.assertEqual(rules(src), [])

    def test_attributes_a_person_reads(self):
        self.assertEqual(rules('<img alt="A stunning cake">'), ["banned-word"])
        self.assertEqual(rules('<input placeholder="Unlock savings">'), ["banned-word"])
        self.assertEqual(rules('<meta name="description" content="Seamless moves">'), ["banned-word"])
        self.assertEqual(rules('<a class="robust" href="/x">Go</a>'), [], "class names aren't words")

    def test_macro_arguments_are_words(self):
        self.assertEqual(rules('{{ ui.button("Submit", kind="primary") }}'), ["button-verb"])
        self.assertEqual(rules('{{ ui.empty("Oops! No orders", "x") }}'), ["opener", "exclamation"])
        self.assertEqual(rules('{{ ui.chip("ok", "ok") }}{% if state == "seamless" %}{% endif %}'), [])

    def test_json_strings_but_not_notes(self):
        src = '{"_about": "seamless notes for editors", "a": {"b": "Rest assured, it works."}, "id": "robust-1"}'
        self.assertEqual(rules(src, "w.json"), ["banned-phrase"])

    def test_js_prose_literals_only(self):
        src = 'const k = "data-state"; toast("Something went wrong, try again"); // Oops!\nel.className = "btn btn-primary";'
        self.assertEqual(rules(src, "i.js"), ["banned-phrase"])

    def test_dart_prose_literals(self):
        src = "final t = Text('Something went wrong, try again'); final k = 'decisions';"
        self.assertEqual(rules(src, "screen.dart"), ["banned-phrase"])

    def test_markdown_skips_code(self):
        src = "# Our Amazing New Feature Set\n\nPay KSh 2,400.\n\n```\nlet x = a! = b;\n```\n"
        self.assertEqual(rules(src, "m.md"), ["title-case"])


class Rules(unittest.TestCase):
    def test_banned_words_and_phrases(self):
        self.assertEqual(rules("<p>We leverage AI to elevate your store.</p>"), ["banned-word", "banned-word"])
        self.assertEqual(rules("<p>Your journey starts here</p>"), ["banned-word", "banned-phrase"])
        self.assertEqual(rules("<p>Oops! Something went wrong.</p>"), ["banned-phrase", "opener", "exclamation"])
        self.assertEqual(rules("<p>Pending approval</p>"), ["banned-phrase"])

    def test_dashes(self):
        self.assertEqual(rules("<p>Fast — and safe</p>"), ["em-dash"])
        self.assertEqual(rules("<p>Open 9–5, or 7:00–20:00, ages 3–6</p>"), [])
        self.assertEqual(rules("<p>Mon–Sat 06:30–19:00, Dec–Jan</p>"), [], "ranges of days and months")
        self.assertEqual(rules("<p>Fast–and safe</p>"), ["en-dash"])
        self.assertEqual(rules("<p>7 Nov – 6 Dec, scenarios S1–S19</p>"), [], "date and code ranges")
        self.assertEqual(rules("<td>{% if x %}{{ x }}{% else %}—{% endif %}</td>"), [], "an empty cell's dash")
        self.assertEqual(rules("<td>{% if r %}{{ a }} of {{ b }}{% else %}—{% endif %}</td>"), [])
        self.assertEqual(rules("<p>{% if r %}Ready — go{% endif %}</p>"), ["em-dash"])

    def test_exclamations_and_emoji(self):
        self.assertEqual(rules("<p>Your site is live!</p>"), ["exclamation"])
        self.assertEqual(rules("<p>Congratulations 🎉</p>"), ["emoji"])
        self.assertEqual(rules('<span aria-hidden="true">☀</span>'), [], "the time pill's sun is a symbol")

    def test_title_case_in_headings_buttons_and_titles(self):
        self.assertEqual(rules("<h1>Read The Mission Report Now</h1>"), ["title-case"])
        self.assertEqual(rules("<button>Approve The Cake Order</button>"), ["title-case"])
        self.assertEqual(rules("<h2>Read the mission report</h2>"), [])
        self.assertEqual(rules("<h2>Tana Valley Times</h2>"), [], "a name stays a name")
        self.assertEqual(rules("<h3>Day Time and Night Time</h3>"), [], "feature names keep capitals")
        self.assertEqual(rules("<h3>Held for Wanjiru's OK</h3>"), [], "acronyms aren't Title Case")
        self.assertEqual(rules("<p>Read The Mission Report Now</p>"), [], "only headings, buttons, titles")

    def test_shapes(self):
        self.assertEqual(rules("<p>It's not just a website, it's a team.</p>"), ["not-x-but-y"])
        self.assertEqual(rules("<p>No forms. No waiting. Just answers.</p>"), ["fragments"])
        self.assertEqual(rules("<p>Sat 7 Nov. 3 orders. KSh 8,400.</p>"), [], "facts with numbers are fine")
        self.assertEqual(rules("<h2>Ready to grow?</h2>"), ["self-answering"])
        self.assertEqual(rules("<p>Want to see it find a real infection? Try the fixture.</p>"), [])

    def test_visitor_only_rules(self):
        self.assertEqual(rules("<p>Chat with our bot</p>", visitor=True), ["visitor-term"])
        self.assertEqual(rules("<p>Chat with our bot</p>", visitor=False), [])
        self.assertEqual(rules("<p>Pay KES 2400 now</p>", visitor=True), ["currency"])
        self.assertEqual(rules("<p>Pay KSh 2,400 now</p>", visitor=True), [])
        self.assertEqual(rules("<p>Pay KSh 2400</p>", visitor=True), ["currency"])

    def test_buttons_are_verbs(self):
        self.assertEqual(rules('<input type="submit" value="Proceed">'), ["button-verb"])
        self.assertEqual(rules("<button>Approve</button>"), [])


class Cli(unittest.TestCase):
    def run_lint(self, files: dict, allow: str = ""):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name, body in files.items():
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(body, encoding="utf-8")
            allow_path = root / "allow"
            allow_path.write_text(allow, encoding="utf-8")
            code = (
                "import sys, importlib.machinery, importlib.util\n"
                f"l = importlib.machinery.SourceFileLoader('cl', {str(LINT)!r})\n"
                "s = importlib.util.spec_from_loader('cl', l); m = importlib.util.module_from_spec(s); sys.modules['cl'] = m; l.exec_module(m)\n"
                f"m.ALLOW_FILE = __import__('pathlib').Path({str(allow_path)!r})\n"
                "sys.exit(m.main(sys.argv[1:]))\n"
            )
            return subprocess.run([sys.executable, "-c", code, *[str(root / n) for n in files]],
                                  capture_output=True, text=True, cwd=d)

    def test_exit_codes_and_allow_entries(self):
        bad = {"a/page.html": "<p>A seamless move!</p>"}
        r = self.run_lint(bad)
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("banned-word", r.stdout)
        self.assertIn("exclamation", r.stdout)
        r = self.run_lint(bad, "*/page.html | * | * | A fixture that tests the lint itself\n")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        r = self.run_lint({"a/page.html": "<p>Pay KSh 2,400.</p>"})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_allow_entries_need_a_reason_and_a_real_rule(self):
        r = self.run_lint({"p.html": "<p>ok</p>"}, "*.html | em-dash | — | ok\n")
        self.assertEqual(r.returncode, 2)
        r = self.run_lint({"p.html": "<p>ok</p>"}, "*.html | no-such-rule | x | because the words say so\n")
        self.assertEqual(r.returncode, 2)

    def test_a_folder_yields_only_files_with_words(self):
        r = self.run_lint({
            "crate/src/lib.rs": 'assert!(x); println!("Oops!");',
            "crate/static/a.css": ".x { content: \"Oops!\" }",
            "crate/templates/p.html": "<p>Pay KSh 2,400.</p>",
            "crate/Cargo.toml": "# Owner: the owner journey, a seamless crate\n",
        })
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_seed_records_are_read_line_by_line(self):
        self.assertEqual(rules('{"note": "Paid. Thanks!"}\n{"note": "Rest assured."}\n', "orders.jsonl"),
                         ["exclamation", "banned-phrase"])

    def test_the_repo_is_clean(self):
        r = subprocess.run([str(LINT)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
