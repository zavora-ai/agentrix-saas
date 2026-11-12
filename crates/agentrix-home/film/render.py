"""Renders the home page film from split.html, frame by frame, then encodes it with ffmpeg.

  python3 render.py [--f kilele|classy|tana|salon] --film film|loop|hero --out <dir> [--fps 30] [--workers 4] [--start 0 --end 9999]

'hero' is the 12-second cut for the home page's hero: the loop without its end card, bigger words.
For the web, encode film/out/web/hero.mp4 and film.mp4 at 1280x720 (H.264, CRF 27 and 25):
node serves that directory as AGENTRIX_HOME_FILM_DIR (see src/film.rs).

Needs the worktree served on :19030 (python3 -m http.server 19030 from the worktree root).
Writes <out>/<film>.mp4 (H.264, for every browser), <out>/<film>.webm (VP9, smaller) and
<out>/<film>-poster.jpg. Frames go to <out>/frames-<film>/ and can be deleted afterwards.
"""

import argparse
import asyncio
import math
import os
import shutil
import subprocess
import time

from cdp import Browser, Page

URL = "http://127.0.0.1:19030/crates/agentrix-home/film/split.html?f={f}&film={film}"


async def worker(n, f, film, frames, fps, out, profile_root):
    with Browser(os.path.join(profile_root, f"w{n}"), port=9340 + n, width=1920, height=1080) as browser:
        page = await Page.open(browser)
        await page.viewport(1920, 1080, 1)
        await page.goto(URL.format(f=f, film=film), settle=0.5)
        await page.eval("window.ready")
        for i in frames:
            await page.eval(f"frame({i / fps:.4f})")
            await page.shot(os.path.join(out, f"{i:05d}.jpg"), fmt="jpeg", quality=93)
        await page.close()


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--f", default="kilele", help="which film: films/<f>.js")
    ap.add_argument("--film", default="film", help="which cut: film, loop or hero")
    ap.add_argument("--out", required=True)
    ap.add_argument("--fps", type=int, default=30)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--start", type=float, default=0)
    ap.add_argument("--end", type=float, default=1e9)
    ap.add_argument("--no-encode", action="store_true")
    args = ap.parse_args()

    name = args.film if args.f == "kilele" else f"{args.f}-{args.film}"
    frames_dir = os.path.join(args.out, f"frames-{name}")
    os.makedirs(frames_dir, exist_ok=True)
    # Ask the page how long the film is.
    with Browser(os.path.join(args.out, ".chrome-probe"), port=9339) as b:
        p = await Page.open(b)
        await p.goto(URL.format(f=args.f, film=args.film), settle=0.3)
        duration = await p.eval("window.DURATION")
        await p.close()
    first = int(args.start * args.fps)
    last = int(min(args.end, duration) * args.fps)
    todo = list(range(first, last))
    chunk = math.ceil(len(todo) / args.workers)
    began = time.time()
    await asyncio.gather(*[
        worker(n, args.f, args.film, todo[n * chunk:(n + 1) * chunk], args.fps, frames_dir, os.path.join(args.out, ".chrome"))
        for n in range(args.workers) if todo[n * chunk:(n + 1) * chunk]
    ])
    print(f"{len(todo)} frames in {time.time() - began:.0f} s ({duration} s film)")
    if args.no_encode:
        return
    src = ["-framerate", str(args.fps), "-start_number", str(first), "-i", os.path.join(frames_dir, "%05d.jpg")]
    mp4 = os.path.join(args.out, f"{name}.mp4")
    webm = os.path.join(args.out, f"{name}.webm")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *src, "-c:v", "libx264", "-preset", "slow", "-crf", "21",
                    "-pix_fmt", "yuv420p", "-movflags", "+faststart", mp4], check=True)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *src, "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "36",
                    "-row-mt", "1", "-pix_fmt", "yuv420p", webm], check=True)
    poster = os.path.join(args.out, f"{name}-poster.jpg")
    shutil.copy(os.path.join(frames_dir, f"{first + min(len(todo) - 1, int(3 * args.fps)):05d}.jpg"), poster)
    for f in (mp4, webm, poster):
        print(f, os.path.getsize(f))


if __name__ == "__main__":
    asyncio.run(main())
