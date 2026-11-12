"""Builds the PayPal entry's film from beats.json: slides, live captures, narration, captions.

    python3 build.py [--out OUT.mp4] [--only b04,b07] [--no-voice] [--final]

- Slides are the sections of slides.html (pic "slide:NAME"), rendered by headless Chrome.
- Clips are files in $AGX_VIDEO_CLIPS (default clips/ beside this file), recorded by rec.py.
  A missing clip falls back to the beat's `fallback` slide with a "Live capture to come" badge,
  and the build says so; a final cut must have none (--final refuses to build with one).
- Narration: ../../submission/video/speak.py (a Vultr text-to-speech model), cached by text.
- Captions, the "Live" badge and the presenter chip: overlay.html, rendered with a transparent
  background.
Work files go to $AGX_VIDEO_WORK (default: a folder in the system temp dir) and are small.
"""

import argparse
import asyncio
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.parse
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../../../crates/agentrix-home/film"))
from cdp import Browser, Page  # noqa: E402

W, H, FPS = 1920, 1080, 30
WORK = os.environ.get("AGX_VIDEO_WORK", os.path.join(tempfile.gettempdir(), "agx-saas-video"))
CLIPS = os.environ.get("AGX_VIDEO_CLIPS", os.path.join(HERE, "clips"))
SPEAK = os.path.join(HERE, "../../submission/video/speak.py")
VOICE = os.environ.get("AGX_VIDEO_VOICE", "")
PRESENTER = ["The Chief", "Agentrix's AI agent, presenting"]
LEAD, TAIL = 0.35, 0.5


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"build: {' '.join(cmd[:6])}… failed:\n{r.stderr[-2000:]}")
    return r.stdout


def slides(names):
    """Renders the named sections of slides.html to PNGs (again when slides.html changes)."""
    out = os.path.join(WORK, "slides")
    os.makedirs(out, exist_ok=True)
    src = os.path.join(HERE, "slides.html")
    stamp = os.path.join(out, ".stamp")
    mtime = str(os.path.getmtime(src))
    fresh = os.path.exists(stamp) and open(stamp).read() == mtime
    todo = [n for n in names if not fresh or not os.path.exists(os.path.join(out, f"{n}.png"))]
    if todo:
        async def render():
            with Browser(os.path.join(WORK, "chrome-slides"), port=9432, width=W, height=H) as b:
                page = await Page.open(b)
                await page.viewport(1280, 720, 1.5)
                for n in todo:
                    await page.goto("file://" + src + "?s=" + n, settle=0.3)
                    await page.eval("window.ready")
                    await page.shot(os.path.join(out, f"{n}.png"), fmt="png")
                await page.close()
        asyncio.run(render())
        open(stamp, "w").write(mtime)
    return out


def voice(text):
    os.makedirs(os.path.join(WORK, "voice"), exist_ok=True)
    path = os.path.join(WORK, "voice", hashlib.sha256(text.encode()).hexdigest()[:16] + ".wav")
    if not os.path.exists(path):
        run([sys.executable, "-I", SPEAK, path, text] + (["--voice", VOICE] if VOICE else []))
    with wave.open(path) as w:
        return path, w.getnframes() / w.getframerate()


def chunks(text, limit=96):
    parts = re.split(r"(?<=[.?!])\s+", text.strip())
    out = []
    for p in parts:
        if out and len(out[-1]) + len(p) + 1 <= limit and len(p) < 40:
            out[-1] += " " + p
        else:
            out.append(p)
    final = []
    for c in out:  # break long sentences at a comma near the middle
        while len(c) > limit + 30:
            cut = c.rfind(", ", 0, limit + 10)
            if cut < 30:
                break
            final.append(c[: cut + 1])
            c = c[cut + 2:]
        final.append(c)
    return final


async def overlays(jobs):
    """jobs: {path: query dict}. Renders each missing PNG with one Chrome."""
    todo = {p: q for p, q in jobs.items() if not os.path.exists(p)}
    if not todo:
        return
    with Browser(os.path.join(WORK, "chrome"), port=9431, width=W, height=H) as b:
        page = await Page.open(b)
        await page.viewport(W, H, 1)
        await page.send("Emulation.setDefaultBackgroundColorOverride", {"color": {"r": 0, "g": 0, "b": 0, "a": 0}})
        base = "file://" + os.path.join(HERE, "overlay.html")
        for path, q in todo.items():
            await page.goto(base + "?" + urllib.parse.urlencode(q), settle=0.1)
            await page.eval("window.ready")
            await page.shot(path, fmt="png")
        await page.close()


def png(q):
    os.makedirs(os.path.join(WORK, "overlay"), exist_ok=True)
    return os.path.join(WORK, "overlay", hashlib.sha256(json.dumps(q, sort_keys=True).encode()).hexdigest()[:16] + ".png")


def segment(beat, i, sl, no_voice):
    say = beat.get("say", "")
    if no_voice or not say:
        vpath, vdur = None, len(say.split()) / 2.4 if say else 0
    else:
        vpath, vdur = voice(say)
    dur = max(beat.get("min", 0), vdur + LEAD + TAIL)
    pic, badge, missing = beat["pic"], beat.get("badge"), False
    inputs, vf = [], []
    if pic.startswith("clip:"):
        clip = os.path.join(CLIPS, pic[5:])
        if os.path.exists(clip):
            start, speed = beat.get("start", 0), beat.get("speed", 1)
            if beat.get("fit") == "clip":
                length = float(run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", clip]))
                end = min(length, beat.get("end", length))
                dur = max(dur, (end - start) / speed)
            if beat.get("fit") == "voice":  # the whole clip, sped up (at most 1.6x) to end with the narration
                length = float(run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", clip]))
                end = min(length, beat.get("end", length))
                speed = min(1.6, max(1.0, (end - start) / dur))
                dur = max(dur, (end - start) / speed)
            inputs += ["-ss", str(start), "-i", clip]
            crop = beat.get("crop")  # "w:h:x:y" in the clip's pixels
            vf.append((f"crop={crop}," if crop else "") + f"setpts=(PTS-STARTPTS)/{speed},scale=1728:972:force_original_aspect_ratio=decrease,pad={W}:{H}:(ow-iw)/2:84+(972-ih)/2:color=0x1f1d3d,fps={FPS},tpad=stop_mode=clone:stop_duration={dur + 1}")
        else:
            missing = True
            pic = "slide:" + str(beat.get("fallback", "title"))
    if pic.startswith("slide:"):
        img = os.path.join(sl, f"{pic[6:]}.png")
        inputs += ["-loop", "1", "-framerate", str(FPS), "-i", img]
        vf.append(f"scale={W}:{H},fps={FPS}")
    # overlays
    ov = []
    if badge:
        ov.append((png({"kind": "badge", "text": badge[0], "sub": badge[1] if len(badge) > 1 else ""}), 0, dur))
    if beat.get("presenter", True):
        ov.append((png({"kind": "presenter", "text": PRESENTER[0], "sub": PRESENTER[1]}), 0, dur))
    if beat.get("speed_label"):
        ov.append((png({"kind": "speed", "text": beat["speed_label"]}), 0, dur))
    if missing:
        ov.append((png({"kind": "speed", "text": "Live capture to come: " + beat["pic"][5:]}), 0, dur))
    if say and beat.get("captions", True):
        parts = chunks(say)
        total = sum(len(p) for p in parts)
        t = LEAD
        span = max(vdur, 0.1)
        for p in parts:
            d = span * len(p) / total
            ov.append((png({"kind": "caption", "text": p}), t, t + d + 0.15))
            t += d
    return dict(beat=beat, inputs=inputs, vf=vf, ov=ov, vpath=vpath, dur=dur, missing=missing, index=i)


def encode(seg, out):
    beat, dur = seg["beat"], seg["dur"]
    args = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"] + seg["inputs"]
    n = 1
    for p, _, _ in seg["ov"]:
        args += ["-i", p]
        n += 1
    if seg["vpath"]:
        args += ["-i", seg["vpath"]]
        a_idx = n
    else:
        args += ["-f", "lavfi", "-t", str(dur), "-i", "anullsrc=r=48000:cl=stereo"]
        a_idx = n
    graph = [f"[0:v]{seg['vf'][0]},trim=duration={dur},setpts=PTS-STARTPTS[v0]"]
    last = "v0"
    for k, (_, a, b) in enumerate(seg["ov"]):
        graph.append(f"[{last}][{k + 1}:v]overlay=0:0:enable='between(t,{a:.2f},{b:.2f})'[v{k + 1}]")
        last = f"v{k + 1}"
    fade = 0.22
    graph.append(f"[{last}]fade=t=in:st=0:d={fade},fade=t=out:st={dur - fade:.2f}:d={fade},format=yuv420p[vout]")
    if seg["vpath"]:
        graph.append(f"[{a_idx}:a]aresample=48000,pan=stereo|c0=c0|c1=c0,adelay={int(LEAD * 1000)}|{int(LEAD * 1000)},apad,atrim=duration={dur}[aout]")
    else:
        graph.append(f"[{a_idx}:a]atrim=duration={dur}[aout]")
    args += ["-filter_complex", ";".join(graph), "-map", "[vout]", "-map", "[aout]", "-t", f"{dur:.3f}",
             "-c:v", "libx264", "-preset", "medium", "-crf", "19", "-r", str(FPS), "-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-ac", "2", out]
    run(args)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(os.path.dirname(CLIPS.rstrip("/")), "agentrix-paypal-demo.mp4"))
    ap.add_argument("--only")
    ap.add_argument("--no-voice", action="store_true")
    ap.add_argument("--final", action="store_true", help="refuse to build while a live capture is missing")
    ap.add_argument("--beats", default=os.path.join(HERE, "beats.json"), help="the beat list to build")
    a = ap.parse_args()
    os.makedirs(WORK, exist_ok=True)
    beats = json.load(open(a.beats))["beats"]
    if a.only:
        keep = set(a.only.split(","))
        beats = [b for b in beats if b["id"] in keep]
    sl = slides(sorted({x["pic"][6:] for x in beats if x["pic"].startswith("slide:")} | {str(x["fallback"]) for x in beats if x.get("fallback")}))
    segs = []
    for i, b in enumerate(beats):
        segs.append(segment(b, i, sl, a.no_voice))
    asyncio.run(overlays({p: OVERLAY_QUERIES[p] for s in segs for p, _, _ in s["ov"]}))
    missing = [s["beat"]["id"] + " (" + s["beat"]["pic"] + ")" for s in segs if s["missing"]]
    if missing and a.final:
        sys.exit("build: live captures missing: " + ", ".join(missing))
    parts = []
    t = 0.0
    for s in segs:
        p = os.path.join(WORK, f"seg-{s['beat']['id']}.mp4")
        encode(s, p)
        parts.append(p)
        print(f"{s['beat']['id']}  {t // 60:.0f}:{t % 60:05.2f}  {s['dur']:5.1f} s  {s['beat']['pic']}{'  MISSING, fallback slide' if s['missing'] else ''}")
        t += s["dur"]
    lst = os.path.join(WORK, "concat.txt")
    with open(lst, "w") as f:
        f.writelines(f"file '{p}'\n" for p in parts)
    run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", lst,
         "-c:v", "copy", "-af", "loudnorm=I=-16:TP=-1.5:LRA=11", "-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-movflags", "+faststart", a.out])
    print(f"{a.out}: {t // 60:.0f} min {t % 60:.1f} s" + (f"; missing live captures: {', '.join(missing)}" if missing else ""))


OVERLAY_QUERIES = {}
_png = png


def png(q):  # noqa: F811 (records the query so main can render it)
    p = _png(q)
    OVERLAY_QUERIES[p] = q
    return p


if __name__ == "__main__":
    main()
