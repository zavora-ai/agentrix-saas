"""Narration through Vultr Serverless Inference's text-to-speech (qwen3-tts-12hz-1.7b).

    python3 -I speak.py OUT.wav "words to say" [--voice aiden] [--instructions "..."]

The key is read from the repository's .env (VULTR_INFERENCE_API_KEY), parsed, never sourced or
printed. Vultr streams base64 PCM (24 kHz, 16-bit mono) as SSE; this writes it as a WAV file and
prints the seconds of audio and the cost at $0.00028 a second.
"""
import argparse
import base64
import json
import os
import pathlib
import sys
import urllib.request
import wave

ENDPOINT = "https://api.vultrinference.com/v1/audio/speech"


def key():
    if os.environ.get("VULTR_INFERENCE_API_KEY"):
        return os.environ["VULTR_INFERENCE_API_KEY"]
    here = pathlib.Path(__file__).resolve()
    for parent in here.parents:
        env = parent / ".env"
        if env.is_file() and (parent / ".git").exists() and (parent / "crates").is_dir() and (parent / ".git").is_dir():
            for line in env.read_text().splitlines():
                if line.startswith("VULTR_INFERENCE_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"').strip("'")
    sys.exit("speak: no VULTR_INFERENCE_API_KEY in the environment or the main checkout's .env")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("text")
    ap.add_argument("--voice", default="aiden")
    ap.add_argument("--instructions", default="A calm, warm, confident narrator for a product demo. Clear diction, an even, unhurried pace.")
    a = ap.parse_args()
    body = {"model": "qwen3-tts-12hz-1.7b", "input": a.text, "voice": a.voice, "instructions": a.instructions,
            "language": "English", "response_format": "pcm", "stream_format": "sse"}
    req = urllib.request.Request(ENDPOINT, data=json.dumps(body).encode(), method="POST",
                                 headers={"Authorization": "Bearer " + key(), "Content-Type": "application/json",
                                          "Accept": "text/event-stream", "User-Agent": "agentrix-submission/1"})
    pcm = bytearray()
    with urllib.request.urlopen(req, timeout=180) as r:
        for raw in r:
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if not data or data == "[DONE]":
                continue
            ev = json.loads(data)
            if ev.get("type") == "speech.audio.delta":
                pcm += base64.b64decode(ev.get("delta") or ev.get("audio") or "")
    with wave.open(a.out, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(24000); w.writeframes(bytes(pcm))
    secs = len(pcm) / 48000
    print(f"{a.out}: {secs:.1f} s, ${secs * 0.00028:.4f}")


if __name__ == "__main__":
    main()
