// Uploaded assets take precedence over the demo sounds.
export class AudioPlayer {
  #context;
  #master;
  #bgm;
  #timer;
  #voices = new Set();
  #enabled = true;

  unlock() {
    if (!this.#context) {
      this.#context = new AudioContext();
      this.#master = this.#context.createGain();
      this.#master.gain.value = this.#enabled ? 1 : 0;
      this.#master.connect(this.#context.destination);
    }
    void this.#context.resume().catch(() => {});
  }

  setEnabled(enabled) {
    this.#enabled = enabled;
    if (this.#master) this.#master.gain.value = enabled ? 1 : 0;
    for (const voice of this.#voices) voice.muted = !enabled;
  }

  startBgm(url, demo) {
    this.stopBgm();
    if (url) {
      this.#bgm = this.#playFile(url, true);
    } else if (demo && this.#context) {
      const notes = [261.63, 329.63, 392, 329.63, 293.66, 349.23, 440, 349.23];
      let index = 0;
      const play = () => {
        this.#bgm = this.#tone(notes[index++ % notes.length], 0.42, 0.075, "sine");
      };
      play();
      this.#timer = setInterval(play, 480);
    }
  }

  stopBgm() {
    clearInterval(this.#timer);
    if (this.#bgm instanceof HTMLAudioElement) {
      this.#bgm.pause();
      this.#voices.delete(this.#bgm);
    } else {
      this.#bgm?.stop();
    }
    this.#bgm = undefined;
  }

  playSe(url, demo) {
    if (url) this.#playFile(url, false);
    else if (demo && this.#context) this.#tone(67, 0.38, 0.15, "triangle");
  }

  reset() {
    this.stopBgm();
    for (const voice of this.#voices) voice.pause();
    this.#voices.clear();
  }

  #playFile(url, loop) {
    const voice = new Audio(url);
    voice.loop = loop;
    voice.muted = !this.#enabled;
    this.#voices.add(voice);
    voice.addEventListener("ended", () => this.#voices.delete(voice), { once: true });
    void voice.play().catch(() => this.#voices.delete(voice));
    return voice;
  }

  #tone(frequency, duration, volume, shape) {
    const oscillator = this.#context.createOscillator();
    const envelope = this.#context.createGain();
    const now = this.#context.currentTime;
    oscillator.type = shape;
    oscillator.frequency.value = frequency;
    envelope.gain.value = volume;
    envelope.gain.exponentialRampToValueAtTime(0.001, now + duration);
    oscillator.connect(envelope).connect(this.#master);
    oscillator.start(now);
    oscillator.stop(now + duration);
    oscillator.onended = () => {
      oscillator.disconnect();
      envelope.disconnect();
    };
    return oscillator;
  }
}
