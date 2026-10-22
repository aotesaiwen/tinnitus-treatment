import { calibration } from './compensation.js';

export const defaults = Object.freeze({ bass: 0, brightness: 0, background: false, backgroundVolume: 30, volume: 65, mode: 'processed' });
export const storageKey = 'listening-room:repertoire-settings:v1';
export function sanitize(value) {
  const clean = { ...defaults };
  if (!value || typeof value !== 'object') return clean;
  for (const [key, min, max] of [['bass', -6, 6], ['brightness', -6, 6], ['backgroundVolume', 0, 100], ['volume', 0, 100]]) {
    if (typeof value[key] === 'number' && Number.isFinite(value[key])) clean[key] = Math.round(Math.min(max, Math.max(min, value[key])));
  }
  clean.background = value.background === true;
  clean.mode = value.mode === 'original' ? 'original' : 'processed';
  return clean;
}

export function compensationDb(bass, brightness) {
  const x = (bass + 6) / 3, y = (brightness + 6) / 3;
  const i = Math.min(3, Math.floor(x)), j = Math.min(3, Math.floor(y));
  const tx = x - i, ty = y - j, a = calibration.compensation_db;
  return (a[i][j]*(1-ty) + a[i][j+1]*ty)*(1-tx) + (a[i+1][j]*(1-ty) + a[i+1][j+1]*ty)*tx;
}

// A finite ramp reaches exact silence, including after repeated rapid changes.
export function ramp(param, value, context, seconds = .08) {
  const now = context.currentTime;
  if (param.cancelAndHoldAtTime) param.cancelAndHoldAtTime(now);
  else {
    const current = param.value;
    param.cancelScheduledValues(now);
    param.setValueAtTime(current, now);
  }
  param.linearRampToValueAtTime(value, now + seconds);
}

export function pinkBuffer(context) {
  // Deterministic Paul Kellet pink-noise approximation. One reusable 12 s loop.
  const length = Math.round(context.sampleRate * 12), overlap = Math.round(context.sampleRate * .5);
  const raw = new Float32Array(length + overlap);
  let seed = 654321, b0 = 0, b1 = 0, b2 = 0, b3 = 0, b4 = 0, b5 = 0, b6 = 0;
  for (let i = 0; i < raw.length; i++) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    const w = seed / 2147483648 - 1;
    b0 = .99886*b0 + w*.0555179; b1 = .99332*b1 + w*.0750759;
    b2 = .969*b2 + w*.153852; b3 = .8665*b3 + w*.3104856;
    b4 = .55*b4 + w*.5329522; b5 = -.7616*b5 - w*.016898;
    raw[i] = b0+b1+b2+b3+b4+b5+b6+w*.5362; b6 = w*.115926;
  }
  const buffer = context.createBuffer(1, length, context.sampleRate), data = buffer.getChannelData(0);
  data.set(raw.subarray(overlap));
  // End blends into the contiguous prefix, avoiding a discontinuity on wrap.
  for (let i = 0; i < overlap; i++) {
    const t = i / overlap;
    data[length-overlap+i] = raw[length+i]*Math.cos(t*Math.PI/2) + raw[i]*Math.sin(t*Math.PI/2);
  }
  const mean = data.reduce((a, b) => a+b, 0) / length;
  let energy = 0;
  for (let i = 0; i < length; i++) { data[i] -= mean; energy += data[i]*data[i]; }
  const scale = .12 / Math.sqrt(energy/length);
  for (let i = 0; i < length; i++) data[i] *= scale;
  return buffer;
}

export function createGraph(context, source) {
  const bass = context.createBiquadFilter(), brightness = context.createBiquadFilter();
  bass.type = 'lowshelf'; bass.frequency.value = 250;
  brightness.type = 'highshelf'; brightness.frequency.value = 2500;
  const match = context.createGain(), noiseGain = context.createGain(), master = context.createGain();
  const guard = context.createWaveShaper();
  // Unity below 0.8; smooth saturation above, bounded below 0.95 (-0.45 dBFS).
  // No oversampling/latency: the neutral music remains sample-identical within float error.
  const curve = new Float32Array(65537);
  for (let i = 0; i < curve.length; i++) {
    const x = i / (curve.length-1)*2-1, a = Math.abs(x);
    curve[i] = a <= .8 ? x : Math.sign(x)*(.8+.15*Math.tanh((a-.8)/.15));
  }
  guard.curve = curve;
  noiseGain.gain.value = 0;
  master.gain.value = 0;
  source.connect(bass).connect(brightness).connect(match).connect(guard).connect(master).connect(context.destination);
  const noise = context.createBufferSource();
  noise.buffer = pinkBuffer(context); noise.loop = true;
  noise.connect(noiseGain).connect(guard); noise.start();
  return {
    bass, brightness, match, noise, noiseGain, guard, master,
    apply(settings, audible, immediate = false) {
      const active = settings.mode === 'processed';
      const low = active ? settings.bass : 0, high = active ? settings.brightness : 0;
      const values = [[bass.gain, low], [brightness.gain, high],
        [match.gain, 10 ** (compensationDb(low, high)/20)],
        [noiseGain.gain, active && settings.background && audible ? .22*settings.backgroundVolume/100 : 0],
        [master.gain, settings.volume/100]];
      for (const [param, value] of values) {
        if (immediate) { param.cancelScheduledValues(context.currentTime); param.setValueAtTime(value, context.currentTime); }
        else ramp(param, value, context);
      }
    },
  };
}
