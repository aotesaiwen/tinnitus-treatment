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
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base);
    assert.equal(await page.locator('html').getAttribute('lang'), 'zh-CN');
    assert.equal(await page.locator('audio').count(), 1, 'Only the completed piece has an audio player');
    assert.equal(await page.locator('.pending-status').textContent(), '等待谱文件');
    await page.waitForFunction(() => Number.isFinite(document.querySelector('audio').duration));
    const duration = await page.locator('audio').evaluate(audio => audio.duration);
    assert(Math.abs(duration - record.duration_seconds) < .1, 'Full piece and natural tail are retained');
    await page.locator('#chopin-title').click();
    await page.locator('audio').evaluate(audio => audio.play());
    await page.waitForFunction(() => document.querySelector('audio').currentTime > .1);
    await page.locator('audio').evaluate(audio => { audio.pause(); audio.currentTime = 150; });
    await page.waitForFunction(() => Math.abs(document.querySelector('audio').currentTime - 150) < .2);
    await page.locator('audio').evaluate(audio => audio.play());
    await page.waitForFunction(() => document.querySelector('audio').currentTime > 150.1);
    await page.locator('audio').evaluate(audio => audio.pause());
    assert.equal(await page.locator('#playback-error').isVisible(), false);
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
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ duration, checks: ['full-track playback', 'seek to 150 seconds', 'unaltered source MIDI download', 'explicit pending second score', 'responsive layout', 'navigation to and from blind test'], browserErrors: errors }, null, 2));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
