import fs from 'node:fs';
import path from 'node:path';
import puppeteer from '../cdp_client/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js';

const root = path.resolve(import.meta.dirname, '..');
const baseURL = process.env.THREE_EXAMPLES_URL || 'http://127.0.0.1:8765/examples/';
const outputDirectory = process.env.CHROME_DIAGNOSTIC_DIR || '/tmp/runs/chrome-same-adapter';
const executablePath = process.env.CHROMIUM || '/etc/profiles/per-user/fox/bin/chromium';
const examples = process.argv.slice(2);
if (examples.length === 0) {
  throw new Error('usage: bun scripts/capture_same_adapter_chrome.ts <example> [...]');
}
fs.mkdirSync(outputDirectory, { recursive: true });
const deterministic = fs.readFileSync(path.join(root, 'e2e/deterministic-injection.ts'), 'utf8');
const cleanPage = fs.readFileSync(path.join(root, 'e2e/clean-page.ts'), 'utf8');

const browser = await puppeteer.launch({
  headless: false,
  executablePath,
  args: [
    '--no-sandbox',
    '--enable-unsafe-webgpu',
    '--enable-features=Vulkan',
    '--use-angle=vulkan',
    '--window-size=800,500',
  ],
});

try {
  for (const example of examples) {
    const page = await browser.newPage();
    page.on('pageerror', (error) => console.error(`${example}: ${error.message}`));
    await page.setViewport({ width: 800, height: 500, deviceScaleFactor: 1 });
    await page.evaluateOnNewDocument(deterministic);
    await page.evaluateOnNewDocument(`{
      const seededRandom = Math.random;
      Math.random = () => {
        const caller = new Error().stack.split('\\n')[2]?.trim();
        return caller?.includes('generateUUID') ? Math._random() : seededRandom();
      };
    }`);
    await page.goto(`${baseURL}${example}.html`, {
      waitUntil: 'networkidle0',
      timeout: 120000,
    });
    await page.waitForFunction(() => window.__deterministicFrameCount?.() > 0, {
      timeout: 120000,
    });
    await page.addScriptTag({ content: cleanPage });
    await page.evaluate(() => {
      window._renderStarted = true;
      window.__runDeterministicFrame();
    });
    await new Promise((resolve) => setTimeout(resolve, 250));
    const output = path.join(outputDirectory, `${example}.png`);
    await page.screenshot({ path: output });
    console.log(`${example}\t${output}`);
    await page.close();
  }
} finally {
  await browser.close();
}
