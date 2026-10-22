/* Check a populated complete-piece page, locally or after publication. */
const { chromium } = require('../../app/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

(async () => {
  const base = process.argv[2] || 'http://127.0.0.1:8765/repertoire/';
  const record = JSON.parse(fs.readFileSync(path.join(__dirname, 'repertoire-record.json')));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    await page.addInitScript(() => {
      const probe = window.__audioProbe = { contexts: [], created: {}, mediaSources: 0, taps: [] };
      const Original = window.AudioContext;
      window.AudioContext = class extends Original {
        constructor(...args) { super(...args); probe.contexts.push(this); }
      };
      for (const method of ['createMediaElementSource', 'createBufferSource', 'createGain', 'createBiquadFilter', 'createWaveShaper']) {
        const original = Original.prototype[method];
        Original.prototype[method] = function (...args) {
          probe.created[method] = (probe.created[method] || 0) + 1;
          if (method === 'createMediaElementSource') probe.mediaSources++;
          return original.apply(this, args);
        };
      }
      const connect = AudioNode.prototype.connect;
      AudioNode.prototype.connect = function (target, ...args) {
        if (target instanceof AudioDestinationNode || target instanceof WaveShaperNode) {
          const analyser = this.context.createAnalyser(); analyser.fftSize = 2048;
          connect.call(this, analyser); probe.taps.push({ node: this, analyser, final: target instanceof AudioDestinationNode });
        }
        return connect.call(this, target, ...args);
      };
      probe.level = () => {
        const tap = probe.taps.find(x => x.final);
        if (!tap) return 0;
        const data = new Float32Array(tap.analyser.fftSize); tap.analyser.getFloatTimeDomainData(data);
        return Math.sqrt(data.reduce((sum, x) => sum + x*x, 0) / data.length);
      };
    });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base);
    assert.equal(await page.locator('html').getAttribute('lang'), 'zh-CN');
    assert.equal(await page.locator('audio').count(), 1, 'Only the completed piece has an audio player');
    assert.equal(await page.locator('.pending-status').textContent(), '等待谱文件');
    await page.waitForFunction(() => Number.isFinite(document.querySelector('audio').duration));
    const duration = await page.locator('audio').evaluate(audio => audio.duration);
    assert(Math.abs(duration - record.duration_seconds) < .1, 'Full piece and natural tail are retained');
    assert(await page.locator('audio').evaluate(audio => audio.paused));
    await page.locator('#play-piece').click();
    await page.waitForFunction(() => document.querySelector('audio').currentTime > .1);
    await page.locator('audio').evaluate(audio => { audio.pause(); audio.currentTime = 150; });
    await page.waitForFunction(() => Math.abs(document.querySelector('audio').currentTime - 150) < .2);
    await page.locator('audio').evaluate(audio => audio.play());
    await page.waitForFunction(() => document.querySelector('audio').currentTime > 150.1);
    await page.locator('audio').evaluate(audio => audio.pause());
    assert.equal(await page.locator('#playback-error').isVisible(), false);
    const range = async (id, value) => {
      await page.locator(`#${id}`).fill(String(value));
      await page.locator(`#${id}`).dispatchEvent('input');
    };
    // Keyboard and real range input events exercise the shipped control handlers.
    await page.locator('#brightness').focus();
    await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('#brightness').inputValue(), '1');
    await range('bass', 6); await range('brightness', -4);
    await page.locator('#background').check(); await range('backgroundVolume', 75);
    await range('volume', 80);
    const beforeSwitch = await page.locator('audio').evaluate(a => a.currentTime);
    await page.locator('[name="comparison"][value="original"]').check();
    assert.equal(await page.locator('audio').evaluate(a => a.currentTime), beforeSwitch);
    assert.equal(await page.locator('#bass').inputValue(), '6');
    await page.locator('[name="comparison"][value="processed"]').check();
    await page.locator('#save-settings').click();
    await page.reload();
    assert.equal(await page.locator('#bass').inputValue(), '6');
    assert.equal(await page.locator('#brightness').inputValue(), '-4');
    assert.equal(await page.locator('#background').isChecked(), true);
    assert.equal(await page.locator('#backgroundVolume').inputValue(), '75');
    assert.equal(await page.locator('#volume').inputValue(), '80');
    assert(await page.locator('audio').evaluate(a => a.paused), 'Restore must not autoplay');
    assert.equal(await page.evaluate(() => __audioProbe.contexts.length), 0);
    await page.locator('#restore-settings').click();
    await page.locator('#play-piece').click();
    await page.waitForFunction(() => __audioProbe.level() > .0001);
    // Seek with the user-facing slider, including from keyboard.
    await range('piece-seek', 80);
    await page.locator('#piece-seek').focus(); await page.keyboard.press('ArrowRight');
    await page.waitForFunction(() => document.querySelector('audio').currentTime > 80);
    const nodesBefore = await page.evaluate(() => ({...__audioProbe.created}));
    for (let i = 0; i < 12; i++) {
      const time = await page.locator('audio').evaluate(a => a.currentTime);
      await page.locator('[name="comparison"][value="original"]').check();
      await page.locator('[name="comparison"][value="processed"]').check();
      const after = await page.locator('audio').evaluate(a => a.currentTime);
      assert(after >= time && after-time < 3, 'Comparison preserves running playback position');
      await page.locator('#background').uncheck(); await page.locator('#background').check();
    }
    assert.deepEqual(await page.evaluate(() => __audioProbe.created), nodesBefore, 'Repeated operations reuse all audio nodes');
    assert.equal(await page.evaluate(() => __audioProbe.mediaSources), 1, 'Exactly one music source');
    await range('volume', 0);
    await page.waitForFunction(() => __audioProbe.level() === 0);
    await range('volume', 80);
    await page.waitForFunction(() => __audioProbe.level() > .0001);
    await page.locator('#play-piece').click();
    await page.waitForFunction(() => __audioProbe.contexts[0].state === 'suspended');
    assert(await page.locator('audio').evaluate(a => a.paused));
    assert.equal(await page.evaluate(() => __audioProbe.taps.filter(x=>!x.final)[1].node.gain.value), 0, 'Pause reaches exact background silence');
    await page.locator('#play-piece').click();
    await range('piece-seek', Math.floor((duration - .3)*10)/10);
    await page.waitForFunction(() => document.querySelector('audio').ended);
    await page.waitForFunction(() => __audioProbe.contexts[0].state === 'suspended');
    assert.equal(await page.evaluate(() => __audioProbe.taps.filter(x=>!x.final)[1].node.gain.value), 0, 'End reaches exact background silence');
    await page.locator('#play-piece').click();
    await page.waitForFunction(() => !document.querySelector('audio').paused);
    await page.evaluate(() => dispatchEvent(new PageTransitionEvent('pagehide')));
    await page.waitForFunction(() => document.querySelector('audio').paused && __audioProbe.contexts[0].state === 'suspended');
    // Preserve both blind listening datasets across repertoire reset.
    await page.evaluate(() => {
      localStorage.setItem('listening-room:round-sentinel', 'saved scores');
    });
    await page.locator('#reset-settings').click();
    assert.equal(await page.locator('#bass').inputValue(), '0');
    assert.equal(await page.locator('#brightness').inputValue(), '0');
    assert.equal(await page.locator('#background').isChecked(), false);
    assert.equal(await page.locator('#backgroundVolume').isDisabled(), true);
    assert.equal(await page.locator('#volume').inputValue(), '65');
    assert.equal(await page.evaluate(() => localStorage.getItem('listening-room:round-sentinel')), 'saved scores');
    await page.reload();
    assert.equal(await page.locator('#bass').inputValue(), '0');
    assert(await page.locator('audio').evaluate(a => a.paused));
    const ranged = await page.request.get(new URL('chopin-nocturne-op9-no2.flac', base).href, { headers: { Range: 'bytes=100000-100999' } });
    assert.equal(ranged.status(), 206, 'Host supports audio byte ranges');
    assert.equal((await ranged.body()).length, 1000);
    const midi = await page.request.get(new URL('chopin-nocturne-op9-no2.mid', base).href);
    assert(midi.ok());
    const bytes = await midi.body();
    assert.equal(bytes.subarray(0, 4).toString(), 'MThd');
    assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), record.source_midi_sha256);
    const output = path.resolve('.runtime/previews');
    fs.mkdirSync(output, { recursive: true });
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: path.join(output, `repertoire-${width}.png`), fullPage: true });
    }
    await page.locator('.site-header > a[href="../"]').last().click();
    await page.locator('.sample-item').first().waitFor();
    await page.locator('a[href="repertoire/"]').click();
    await page.locator('#chopin-title').waitFor();
    // Mobile touch-sized controls, with actual interaction as well as layout checks.
    const mobile = await browser.newContext({ viewport: {width:390,height:844}, isMobile:true, hasTouch:true, deviceScaleFactor:2 });
    const phone = await mobile.newPage(); phone.on('pageerror', e => errors.push(e.message));
    await phone.goto(base); await phone.locator('#play-piece').tap();
    await phone.waitForFunction(() => document.querySelector('audio').currentTime > .1);
    await phone.locator('#background').tap();
    await phone.locator('[name="comparison"][value="original"]').check();
    await phone.locator('#play-piece').tap();
    assert(await phone.locator('audio').evaluate(a=>a.paused));
    assert(await phone.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await mobile.close();
    const blocked = await browser.newPage();
    blocked.on('pageerror', e=>errors.push(e.message));
    await blocked.addInitScript(() => { Storage.prototype.getItem = Storage.prototype.setItem = () => {throw new Error('Storage blocked');}; });
    await blocked.goto(base);
    await blocked.locator('#reset-settings').click();
    assert.match(await blocked.locator('#settings-status').textContent(), /未允许保存/);
    await blocked.locator('#play-piece').click();
    await blocked.waitForFunction(()=>document.querySelector('audio').currentTime > .1);
    await blocked.close();
    const fallback = await browser.newPage();
    fallback.on('pageerror', e=>errors.push(e.message));
    await fallback.addInitScript(() => { window.AudioContext = undefined; window.webkitAudioContext = undefined; });
    await fallback.goto(base);
    assert.equal(await fallback.locator('audio').isVisible(), true);
    await fallback.locator('#chopin-title').click();
    await fallback.locator('audio').evaluate(a=>a.play());
    await fallback.waitForFunction(()=>document.querySelector('audio').currentTime > .1);
    await fallback.close();
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ duration, checks: ['full-track playback', 'seek to 150 seconds', 'unaltered source MIDI download', 'explicit pending second score', 'responsive layout', 'navigation to and from blind test', 'actual DSP output and master mute', 'same-position A/B', 'fixed source/node counts after 24 switches', 'pause/end/pagehide background silence', 'save/restore/reset without autoplay', 'storage-denied playback', 'keyboard and mobile touch controls', 'HTTP 206 seek support'], browserErrors: errors }, null, 2));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
