/* Run against a populated demo: node research/listening/check_demo.cjs URL */
const { chromium } = require('../../app/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

(async () => {
  const base = process.argv[2] || 'http://127.0.0.1:8765';
  const output = path.resolve('.runtime/previews');
  fs.mkdirSync(output, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => requests.push(request.url()));
    await page.goto(base);
    await page.locator('.sample-item').first().waitFor();
    const count = await page.locator('.sample-item').count();
    assert(count >= 2, 'A comparison requires multiple real samples');
    const session = () => page.evaluate(() => JSON.parse(localStorage.getItem(Object.keys(localStorage).find(k => k.startsWith('listening-room:')))));
    const initial = await session();
    assert.equal(await page.locator('#reveal').isDisabled(), true);
    assert(!requests.some(url => url.endsWith('/reveal.json')), 'No source identity request before revealing');
    assert.equal(await page.locator('#sample-heading').textContent(), '片段 A');
    await page.screenshot({ path: path.join(output, 'listening-desktop.png'), fullPage: true, animations: 'disabled' });

    await page.locator('#play').click();
    await page.waitForFunction(() => document.querySelector('audio').currentTime > .05);
    await page.locator('#play').click();
    await page.evaluate(() => { document.querySelector('audio').currentTime = 8; });
    await page.locator('.sample-item').nth(1).click();
    await page.waitForFunction(() => Math.abs(document.querySelector('audio').currentTime - 8) < .2);
    await page.locator('.sample-item').first().click();
    for (const criterion of ['clarity', 'music', 'replay']) {
      await page.locator(`input[name="${criterion}"][value="4"]`).check({ force: true });
    }
    const note = '<img src=x onerror="window.unexpectedScript=true"> 中文试听笔记';
    await page.locator('#notes').fill(note);
    const order = (await session()).order;
    await page.reload();
    await page.locator('.sample-item').first().waitFor();
    assert.deepEqual((await session()).order, order, 'Reload must preserve anonymous labels');
    assert.equal(await page.locator('#notes').inputValue(), note);
    for (let i = 1; i < count; i++) {
      await page.locator('.sample-item').nth(i).click();
      await page.locator('#play').click();
      await page.waitForFunction(() => {
        const audio = document.querySelector('audio');
        return !audio.paused && audio.readyState >= 2 && audio.currentTime > .05;
      });
      assert.equal(await page.locator('#player-error').isVisible(), false);
      await page.locator('#play').click();
      await page.locator('#skip').click();
    }
    assert.equal(await page.locator('#reveal').isEnabled(), true);
    assert(!requests.some(url => url.endsWith('/reveal.json')));
    await page.locator('#reveal').click();
    await page.locator('#confirm-action').click();
    await page.locator('#results').waitFor({ state: 'visible' });
    assert.equal(await page.locator('.result-row').count(), count);
    assert.equal(await page.locator('#notes').isDisabled(), true);
    assert.equal(await page.locator('#ratings input').first().isDisabled(), true);
    assert.equal(await page.locator('.result-note').textContent(), note);
    assert.equal(await page.evaluate(() => window.unexpectedScript), undefined);
    const downloaded = page.waitForEvent('download');
    await page.locator('#export').click();
    const download = await downloaded;
    const feedback = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
    assert.equal(feedback.samples.length, count);
    assert.equal(feedback.samples[0].note, note);
    assert(feedback.samples.every(sample => sample.source.label));
    assert.equal(feedback.samples[0].ratings.replay, 4);
    await page.reload();
    await page.locator('#results').waitFor({ state: 'visible' });
    assert.equal(await page.locator('#notes').isDisabled(), true);
    await page.locator('#new-session').click();
    await page.locator('#confirm-action').click();
    await page.locator('#results').waitFor({ state: 'hidden' });
    const fresh = await session();
    assert.notEqual(fresh.session, initial.session);
    assert(Object.values(fresh.answers).every(a => Object.keys(a.ratings).length === 0));
    assert.equal(await page.locator('#sample-heading').textContent(), '片段 A');
    assert.equal(await page.locator('#reveal').isDisabled(), true);

    for (const width of [390, 768]) {
      await page.setViewportSize({ width, height: 844 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `No horizontal overflow at ${width}px`);
      await page.screenshot({ path: path.join(output, `listening-${width}.png`), fullPage: true, animations: 'disabled' });
    }
    assert.deepEqual(errors, [], 'No browser script errors');
    console.log(JSON.stringify({ samples: count, checks: ['actual FLAC playback', 'position-preserving switch', 'source identities deferred', 'rating persistence', 'skip and reveal gate', 'locked ratings', 'literal notes', 'JSON export', 'new round', 'responsive widths'], browserErrors: errors }, null, 2));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
