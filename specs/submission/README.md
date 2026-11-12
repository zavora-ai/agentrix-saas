# The Vultr Agent Rush submission

Challenge 1, Blast Radius Zero. The form closes **Mon 9 Nov 2026, 02:00 EAT**. Owner: W14 (the
submission session); James submits.

| Form item | File | State |
|---|---|---|
| Title, short and long descriptions, mode, categories, technologies, links | [`form.md`](form.md); check the lengths with `python3 specs/submission/count.py` | Ready. Paste the video's upload link |
| Cover image, 16:9 | [`cover/cover.png`](cover/cover.png), 1920 × 1080 | Ready |
| Slides, PDF | [`deck/agentrix-vultr-deck.pdf`](deck/agentrix-vultr-deck.pdf), 19 slides; source [`deck/deck.html`](deck/deck.html) | Ready; the last slide's video card waits on the upload link |
| Demo video, under 5 minutes | `video/agentrix-vultr-demo.mp4` (4:51, kept out of git; upload it). Screen-share cut: [`video/beats-demo.json`](video/beats-demo.json), scenes in [`video/demo.py`](video/demo.py) | Ready to upload |
| README: Vultr services, architecture, use case, how tasks run, containment | The repository's [`README.md`](../../README.md), sections 2, 4, 6 and 9 | Public |
| Repository, demo platform, demo URL | https://github.com/zavora-ai/agentrix-webagent-platform · Other (Vultr) · https://agentrix.website | Ready |

## Rebuild any of it

```sh
specs/submission/render.sh deck     # the PDF, from deck/deck.html
specs/submission/render.sh cover    # the cover, the deck's first slide at 1920 x 1080
python3 specs/submission/video/rec.py home specs/submission/video/clips/home.mp4   # a live capture
python3 specs/submission/video/build.py --final                                    # the video
```

- [`video/rec.py`](video/rec.py) drives headless Chrome on its own profile against the live site
  and streams its screencast into ffmpeg, so a capture runs in real time and leaves no frames on
  disk. The Move, which needs James's passkey, is recorded from his screen with `screencapture`.
- [`video/speak.py`](video/speak.py) speaks the narration through Vultr Serverless Inference
  (`qwen3-tts-12hz-1.7b`, about $0.08 for the whole video). The key is parsed from the main
  checkout's `.env` and never printed.
- [`video/build.py`](video/build.py) puts the beats together. Captions and the "Live" badge are
  drawn by [`video/overlay.html`](video/overlay.html) in the site's own type. `--final` refuses
  to build while any live capture is missing.

## Honesty

Every number in these files comes from a receipt, the usage meter or a dated measurement, and
says which. `TODO(live)`, `[[…]]` and the deck's `LIVE` table mark what the live run on Vultr
routes still has to supply. Nothing is filled in by hand.
