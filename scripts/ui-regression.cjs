// Tests the built UI with deterministic IPC responses; does not open user files.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');

const dist = path.resolve(__dirname, '../dist');
const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PTAM_BROWSER_PATH || (fs.existsSync(chrome) ? chrome : undefined);

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath });
  let passed = 0;
  async function run(name, mode, check) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await context.route('http://ptam2.test/**', route => {
      const relative = new URL(route.request().url()).pathname.slice(1) || 'index.html';
      const file = path.resolve(dist, relative);
      if (!file.startsWith(dist + path.sep) || !fs.existsSync(file)) return route.abort();
      return route.fulfill({ path: file });
    });
    await page.addInitScript(({ mode }) => {
      if (mode === 'background' && !localStorage.getItem('ptam2.settings')) {
        localStorage.setItem('ptam2.settings', JSON.stringify({ backgroundPath: 'Z:/offline/background.png' }));
      }
      const image = (color, half = false) => {
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 64;
        const context = canvas.getContext('2d');
        context.fillStyle = color;
        context.fillRect(0, 0, half ? 32 : 64, 64);
        return canvas.toDataURL();
      };
      const size = mode === 'dense' ? 8 : 256;
      const textures = [1, 2].map(id => ({ id, name: `texture-${id}.png`, path: `C:/fixture/${id}.png`,
        width: size, height: size, format: 'PNG', thumbnailDataUrl: image('#00ff00', true) }));
      const base = { id: 99, name: 'base.png', path: 'C:/fixture/base.png', width: mode === 'dense' ? 8192 : 512,
        height: mode === 'dense' ? 8192 : 256, format: 'PNG', thumbnailDataUrl: image('#ff0000') };
      window.calls = [];
      window.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => {
        window.calls.push({ cmd, args });
        if (cmd === 'get_project_state') return { textures, base: ['plain', 'background'].includes(mode) ? null : base };
        if (cmd === 'remove_textures') { if (window.delayRemove) await new Promise(resolve => window.finishRemove = resolve); return; }
        if (cmd === 'clear_textures' || cmd === 'clear_base_texture') return;
        if (cmd === 'plugin:dialog|open') return args.options.multiple ? ['C:/fixture/new.png'] : 'C:/fixture/new-base.png';
        if (cmd === 'plugin:dialog|save') return 'C:/fixture/atlas.png';
        if (cmd === 'set_base_texture') { await new Promise(resolve => window.finishBase = resolve); return { ...base, id: 100, name: 'new-base.png' }; }
        if (cmd === 'add_textures') { await new Promise(resolve => window.finishAdd = resolve); return { textures: [{ ...textures[0], id: 3 }], errors: [], duplicateCount: 0 }; }
        if (cmd === 'read_background_image') throw 'offline';
        if (cmd === 'build_preview') {
          if (window.delayPreview) await new Promise(resolve => window.finishPreview = resolve);
          return { width: 512, height: 256, previewDataUrl: base.thumbnailDataUrl, placements: [], grid: null };
        }
        if (cmd === 'export_atlas') return { outputPath: 'C:/fixture/atlas.png', jsonPath: 'C:/fixture/atlas.json', width: 512, height: 256,
          format: 'PNG', mode: 'full-encode', elapsedMs: 42, preservedOutsideSlots: false, diagnostics: [] };
        throw new Error(`Unexpected command ${cmd}`);
      } };
    }, { mode });
    try {
      await page.goto('http://ptam2.test/');
      await page.locator('.texture-item').first().waitFor();
      await check(page);
      assert.deepEqual(errors, [], 'No unhandled browser errors');
      console.log(`PASS ${name}`);
      passed++;
    } finally { await context.close(); }
  }
  const tick = page => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const pixels = (page, x, y) => page.locator('#preview-image').evaluate((canvas, { x, y }) => Array.from(canvas.getContext('2d').getImageData(x, y, 1, 1).data), { x, y });
  try {
    await run('gear stays at the right edge while loading', 'base', async p => {
      const before = await p.locator('#settings-button').boundingBox();
      assert.ok(before.x > 1300);
      await p.locator('#base-button').click();
      await p.waitForFunction(() => !!window.finishBase);
      assert.equal((await p.locator('#settings-button').boundingBox()).x, before.x);
      assert.equal(await p.locator('#settings-button').isEnabled(), true);
    });
    await run('Delete does not remove textures from settings or resize input', 'base', async p => {
      await p.locator('.texture-item[data-id="1"]').click();
      await p.locator('#settings-button').click();
      await p.locator('#canvas-mode').selectOption('custom');
      await p.locator('#canvas-width').focus();
      await p.keyboard.press('Delete');
      await p.keyboard.press('Escape');
      await p.locator('#resize-button').click();
      await p.locator('#resize-width').focus();
      await p.keyboard.press('Delete');
      assert.equal(await p.evaluate(() => window.calls.filter(c => c.cmd === 'remove_textures').length), 0);
    });
    await run('removing one texture preserves all other slots', 'base', async p => {
      await p.locator('#auto-place-button').click();
      await p.locator('.texture-item[data-id="1"]').click();
      await p.keyboard.press('Delete');
      await p.waitForFunction(() => document.querySelectorAll('.texture-item').length === 1);
      assert.equal(await p.locator('.slot-cell.occupied').count(), 1);
      assert.equal(await p.locator('.slot-cell').getAttribute('data-slot'), '2');
    });
    await run('selection changes during removal cannot delete the wrong UI item', 'base', async p => {
      await p.locator('.texture-item[data-id="1"]').click();
      await p.evaluate(() => window.delayRemove = true);
      await p.locator('#remove-button').click();
      await p.waitForFunction(() => !!window.finishRemove);
      await p.locator('.texture-item[data-id="2"]').click();
      await p.evaluate(() => window.finishRemove());
      await p.waitForFunction(() => document.querySelectorAll('.texture-item').length === 1);
      assert.equal(await p.locator('.texture-item').getAttribute('data-id'), '2');
      assert.equal(await p.locator('.texture-item.selected').count(), 1);
    });
    await run('transparent replacement clears the base without tinting pixels', 'base', async p => {
      await p.locator('#auto-place-button').click();
      await tick(p);
      assert.deepEqual(await pixels(p, 192, 128), [0, 0, 0, 0]);
      assert.deepEqual(await pixels(p, 64, 128), [0, 255, 0, 255]);
      assert.equal(await p.locator('.slot-cell strong').first().isVisible(), false);
    });
    await run('custom base canvas matches dimensions and transparent padding', 'base', async p => {
      await p.locator('#settings-button').click();
      await p.locator('#canvas-mode').selectOption('1024');
      await p.keyboard.press('Escape');
      await tick(p);
      assert.equal(await p.locator('#canvas-dimensions').innerText(), '1024 × 1024');
      assert.deepEqual(await pixels(p, 700, 700), [0, 0, 0, 0]);
      const grid = await p.locator('#slot-overlay').boundingBox();
      const frame = await p.locator('#atlas-frame').boundingBox();
      assert.ok(Math.abs(grid.width / frame.width - 0.5) < 0.01);
      assert.equal(await p.locator('#preview-button').isVisible(), false);
    });
    await run('native HTML drag into an empty slot and swap occupied slots', 'base', async p => {
      const overlay = p.locator('#slot-overlay');
      const box = await overlay.boundingBox();
      await p.locator('.texture-item[data-id="1"]').dragTo(overlay, { targetPosition: { x: box.width * .25, y: box.height * .5 } });
      await p.waitForFunction(() => document.querySelector('.slot-cell[data-slot="1"]'));
      await p.locator('.texture-item[data-id="2"]').dragTo(overlay, { targetPosition: { x: box.width * .75, y: box.height * .5 } });
      await p.locator('.slot-cell[data-slot="1"]').dragTo(p.locator('.slot-cell[data-slot="2"]'));
      assert.equal(await p.locator('.slot-cell[data-slot="1"]').getAttribute('title'), 'texture-2.png');
      assert.equal(await p.locator('.slot-cell[data-slot="2"]').getAttribute('title'), 'texture-1.png');
    });
    await run('clearing a loading base prevents late restoration', 'base', async p => {
      await p.locator('#base-button').click();
      await p.waitForFunction(() => !!window.finishBase);
      await p.locator('#clear-base-button').click();
      await p.evaluate(() => window.finishBase());
      await tick(p);
      assert.equal(await p.locator('#base-card').isVisible(), false);
      assert.equal(await p.locator('#engine-activity').isVisible(), false);
      assert.equal(await p.locator('#base-button').isEnabled(), true);
    });
    await run('clearing textures cancels pending import', 'plain', async p => {
      await p.locator('#add-button').click();
      await p.waitForFunction(() => !!window.finishAdd);
      await p.locator('#clear-button').click();
      await p.evaluate(() => window.finishAdd());
      await tick(p);
      assert.equal(await p.locator('.texture-item').count(), 0);
      assert.equal(await p.locator('#add-button').isEnabled(), true);
    });
    await run('export-only settings preserve preview and survive reload', 'plain', async p => {
      await p.locator('#preview-button').click();
      await p.waitForFunction(() => !document.querySelector('#atlas-frame').hidden);
      await p.locator('#settings-button').click();
      await p.locator('#export-json').uncheck();
      await p.locator('#export-format').selectOption('bc7-srgb');
      assert.equal(await p.locator('#atlas-frame').isVisible(), true);
      await p.reload();
      await p.locator('#settings-button').click();
      assert.equal(await p.locator('#export-format').inputValue(), 'bc7-srgb');
      assert.equal(await p.locator('#export-json').isChecked(), false);
    });
    await run('stale ordinary preview cannot replace a newer project state', 'plain', async p => {
      await p.evaluate(() => window.delayPreview = true);
      await p.locator('#preview-button').click();
      await p.waitForFunction(() => !!window.finishPreview);
      assert.equal(await p.locator('#settings-button').isEnabled(), true);
      await p.locator('#clear-button').click();
      await p.evaluate(() => window.finishPreview());
      await tick(p);
      assert.equal(await p.locator('#atlas-frame').isVisible(), false);
    });
    await run('offline background keeps the saved path', 'background', async p => {
      await tick(p);
      assert.equal(await p.evaluate(() => JSON.parse(localStorage.getItem('ptam2.settings')).backgroundPath), 'Z:/offline/background.png');
    });
    await run('settings trap keyboard focus and hide irrelevant base options', 'base', async p => {
      await p.locator('#settings-button').click();
      await p.keyboard.press('Shift+Tab');
      assert.equal(await p.evaluate(() => document.activeElement.id), 'panel-opacity');
      await p.keyboard.press('Tab');
      assert.equal(await p.evaluate(() => document.activeElement.id), 'settings-close-button');
      assert.equal(await p.locator('#layout-field').isVisible(), false);
      assert.equal(await p.locator('#padding-field').isVisible(), false);
    });
    await run('million-slot grid uses bounded DOM and still accepts a drop', 'dense', async p => {
      assert.equal(await p.locator('.slot-cell').count(), 0);
      const overlay = p.locator('#slot-overlay');
      const box = await overlay.boundingBox();
      await p.locator('.texture-item[data-id="1"]').dragTo(overlay, { targetPosition: { x: box.width * .9, y: box.height * .9 } });
      assert.equal(await p.locator('.slot-cell').count(), 1);
      assert.ok(Number(await p.locator('.slot-cell').getAttribute('data-slot')) > 900000);
    });
    await run('export report defaults to paths only, details collapsed', 'base', async p => {
      await p.locator('#auto-place-button').click();
      await p.locator('#export-button').click();
      await p.locator('.report-modal').waitFor();
      assert.equal(await p.locator('.export-details').getAttribute('open'), null);
      assert.equal(await p.locator('.report-grid').isVisible(), false);
      assert.equal(await p.locator('.toast.success').count(), 0);
    });
    await run('compact window keeps primary actions reachable', 'base', async p => {
      await p.setViewportSize({ width: 1000, height: 650 });
      await p.locator('#auto-place-button').click();
      for (const id of ['#settings-button', '#add-button', '#base-button', '#export-button']) {
        const box = await p.locator(id).boundingBox();
        assert.ok(box.x >= 0 && box.y >= 0 && box.x + box.width <= 1000 && box.y + box.height <= 650, id);
      }
      if (process.env.PTAM_SCREENSHOT_DIR) {
        await p.screenshot({ path: path.join(process.env.PTAM_SCREENSHOT_DIR, 'ptam2-compact.png') });
        await p.setViewportSize({ width: 1440, height: 900 });
        await p.screenshot({ path: path.join(process.env.PTAM_SCREENSHOT_DIR, 'ptam2-main.png') });
        await p.locator('#settings-button').click();
        await p.screenshot({ path: path.join(process.env.PTAM_SCREENSHOT_DIR, 'ptam2-settings.png'), animations: 'disabled' });
      }
    });
    console.log(`${passed} UI regression tests passed.`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
