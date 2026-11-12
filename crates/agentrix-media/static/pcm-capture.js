// The owner's microphone as PCM16, mono, 16 kHz, in 100 ms frames (Vultr's realtime format).
// Runs in an AudioWorklet; resamples from the device's rate by linear interpolation.
class PcmCapture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.step = sampleRate / 16000;
    this.pos = 0;
    this.prev = 0;
    this.frame = new Int16Array(1600);
    this.n = 0;
  }
  process(inputs) {
    const input = inputs[0] && inputs[0][0];
    if (!input) return true;
    let pos = this.pos;
    while (pos < input.length) {
      const i = Math.floor(pos), t = pos - i;
      const a = i === 0 ? this.prev : input[i - 1], b = input[i];
      const s = Math.max(-1, Math.min(1, a + (b - a) * t));
      this.frame[this.n++] = s < 0 ? s * 0x8000 : s * 0x7fff;
      if (this.n === this.frame.length) {
        this.port.postMessage(this.frame.buffer, [this.frame.buffer]);
        this.frame = new Int16Array(1600);
        this.n = 0;
      }
      pos += this.step;
    }
    this.pos = pos - input.length;
    this.prev = input[input.length - 1];
    return true;
  }
}
registerProcessor("pcm-capture", PcmCapture);
