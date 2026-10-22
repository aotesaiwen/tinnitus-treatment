import { createGraph, defaults, sanitize, storageKey, ramp } from './audio-engine.js';

const $ = selector => document.querySelector(selector);
const player = $('#chopin-audio'), error = $('#playback-error');
const status = $('#settings-status'), playButton = $('#play-piece'), seek = $('#piece-seek');
let settings = { ...defaults }, context, graph, audible = false, suspendTimer, starting = false, playRequest = 0;
const controls = ['bass', 'brightness', 'backgroundVolume', 'volume'];
const AudioContextClass = window.AudioContext || window.webkitAudioContext;

function readSettings() {
  try {
    const value = localStorage.getItem(storageKey);
    settings = sanitize(value ? JSON.parse(value) : defaults);
    status.textContent = value ? '已恢复此浏览器的设置；点击播放开始试听。' : '设置会自动保存在此浏览器。';
  } catch {
    settings = { ...defaults };
    status.textContent = '无法读取本地设置，已使用默认值；仍可正常试听。';
  }
}
function save(message = '已自动保存在此浏览器。') {
  try { localStorage.setItem(storageKey, JSON.stringify(settings)); status.textContent = message; }
  catch { status.textContent = '浏览器未允许保存；本次调节仍然有效。'; }
}
function apply() {
  graph?.apply(settings, audible && !player.paused && !player.ended && !player.seeking);
}
function render() {
  for (const key of controls) {
    const input = $(`#${key}`), value = settings[key];
    input.value = value;
    const label = key === 'bass' || key === 'brightness' ? (value === 0 ? '中性' : `${value > 0 ? '+' : '−'}${Math.abs(value)}`) : `${value}%`;
    $(`#${key}-value`).textContent = label;
    input.setAttribute('aria-valuetext', label);
  }
  $('#background').checked = settings.background;
  $('#backgroundVolume').disabled = !settings.background;
  for (const input of document.querySelectorAll('[name="comparison"]')) input.checked = input.value === settings.mode;
  $('#mode-description').textContent = settings.mode === 'original'
    ? '原版：原始音乐，不加背景声。调节值仍保留，切回处理版即可听到。'
    : '处理版：应用下方音色调节；背景声开启后也参与对比。';
  apply();
}
function reportError(message) { error.textContent = message; error.hidden = false; }
function timeLabel(seconds) {
  if (!Number.isFinite(seconds)) return '0:00';
  return `${Math.floor(seconds/60)}:${String(Math.floor(seconds%60)).padStart(2, '0')}`;
}
function progress() {
  const ready = Number.isFinite(player.duration);
  seek.disabled = !ready; seek.max = ready ? player.duration : 1;
  seek.value = player.currentTime;
  seek.setAttribute('aria-valuetext', `${timeLabel(player.currentTime)}，共 ${timeLabel(player.duration)}`);
  $('#piece-time').textContent = `${timeLabel(player.currentTime)} / ${timeLabel(player.duration)}`;
}
async function unlock() {
  if (!AudioContextClass) return;
  clearTimeout(suspendTimer);
  if (!context) {
    context = new AudioContextClass();
    graph = createGraph(context, context.createMediaElementSource(player));
    graph.apply(settings, false, true);
    context.addEventListener('statechange', () => {
      if (context.state === 'interrupted') player.pause();
    });
  }
  if (context.state !== 'running') await context.resume();
}
function silence() {
  audible = false;
  apply();
  clearTimeout(suspendTimer);
  // Finish the short background fade, then stop processing the reusable graph.
  suspendTimer = setTimeout(() => {
    if (!audible && context?.state === 'running') context.suspend().catch(() => {});
  }, 120);
}
playButton.addEventListener('click', async () => {
  if (!player.paused || starting) { playRequest++; starting = false; player.pause(); silence(); return; }
  const request = ++playRequest;
  starting = true;
  try {
    // Start both operations within the user gesture (including mobile Safari).
    await Promise.all([unlock(), player.play()]);
    if (request !== playRequest) player.pause();
    else error.hidden = true;
  }
  catch {
    if (request === playRequest) { player.pause(); silence(); reportError('播放未能开始，请再次点击播放；也可下载完整音频试听。'); }
  }
  finally { if (request === playRequest) starting = false; }
});
player.addEventListener('play', () => {
  playButton.textContent = '暂停';
  unlock().catch(() => { player.pause(); reportError('声音处理未能启动，请刷新后重试，或下载音频试听。'); });
});
player.addEventListener('playing', () => { audible = true; clearTimeout(suspendTimer); unlock().then(apply).catch(() => player.pause()); });
player.addEventListener('pause', () => { playButton.textContent = '播放'; silence(); });
player.addEventListener('ended', () => { playButton.textContent = '重新播放'; silence(); progress(); });
for (const event of ['waiting', 'seeking', 'emptied']) player.addEventListener(event, silence);
player.addEventListener('seeked', () => {
  progress();
  if (!player.paused && player.readyState >= 3) {
    audible = true; unlock().then(apply).catch(() => player.pause());
  }
});
player.addEventListener('error', () => { player.pause(); silence(); reportError('音频暂时无法载入，请刷新后重试，或使用下面的下载链接。'); });
player.addEventListener('loadeddata', () => { error.hidden = true; });
for (const event of ['timeupdate', 'durationchange', 'loadedmetadata']) player.addEventListener(event, progress);
seek.addEventListener('input', () => { if (Number.isFinite(player.duration)) player.currentTime = Number(seek.value); progress(); });
for (const key of controls) $(`#${key}`).addEventListener('input', event => {
  settings[key] = Number(event.target.value); render(); save();
});
$('#background').addEventListener('change', event => { settings.background = event.target.checked; render(); save(); });
for (const input of document.querySelectorAll('[name="comparison"]')) input.addEventListener('change', () => {
  settings.mode = input.value; render(); save();
});
$('#save-settings').addEventListener('click', () => save('已保存当前设置；下次打开会自动恢复，不会自动播放。'));
$('#restore-settings').addEventListener('click', () => { readSettings(); render(); });
$('#reset-settings').addEventListener('click', () => { settings = { ...defaults }; render(); save('已恢复默认：音色中性、背景声关闭、总音量 65%。'); });
window.addEventListener('pagehide', () => {
  playRequest++; starting = false;
  player.pause(); clearTimeout(suspendTimer); audible = false;
  if (context) {
    ramp(graph.noiseGain.gain, 0, context, 0);
    context.suspend().catch(() => {});
  }
});
// Enhanced controls have their own master volume, including on mobile browsers
// that do not allow software changes to HTMLMediaElement.volume.
readSettings(); render(); progress();
if (AudioContextClass) {
  player.controls = false;
  player.hidden = true;
  $('#listening-controls').hidden = false;
} else {
  reportError('此浏览器不支持声音调节，可用原生播放器听原版，或换用新版浏览器。');
}
