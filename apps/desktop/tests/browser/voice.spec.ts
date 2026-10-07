import { test, expect } from '@playwright/test';
import { pcmWave } from '../../src/features/voice/audio';

const wave = Buffer.from(pcmWave(new Float32Array(1600), 16000));
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    w.calls = 0; w.cancelled = 0;
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: () => 1, unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 3, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'list_tasks') return [];
        if (cmd === 'personal_memory_overview') return {online:false,stats:null,serviceUrl:'http://127.0.0.1:4322'};
        if (cmd === 'model_settings_get') return { baseUrl: 'https://example.com/v1', model: 'custom-chat', useApiKey: false, hasApiKey: false, maxOutputTokens: 1024 };
        if (cmd === 'voice_cancel') { w.cancelled++; return; }
        if (cmd === 'voice_probe') {
          w.calls++;
          w.lastRequest = args.request;
          args.onStage.onmessage({ requestId: args.request.requestId, stage: 'generating' });
          return new Promise(resolve => { w.finishVoice = () => resolve({ requestId: args.request.requestId, transcript: '测试问题', reply: '测试回答', wav: args.request.wav, recognitionMs: 1, generationMs: 2, synthesisMs: 3, inputSeconds: .1, outputSeconds: .1, modelTotalTokens: 4, audioCost: null }); });
        }
        return 1;
      },
    } });
  });
  await page.goto('/?view=panel');
  await page.getByRole('button', { name: '打开语音实验' }).click();
  await page.getByLabel('语音 API 基地址', { exact: true }).fill('https://example.com/v1');
  await page.getByLabel('语音服务需要 API Key', { exact: true }).uncheck();
  await page.getByLabel('识别模型', { exact: true }).fill('custom-asr');
  await page.getByLabel('合成模型', { exact: true }).fill('custom-tts');
  await page.getByLabel('音色编码', { exact: true }).fill('custom-voice');
});

test('selected audio uploads only on action, and stopped late result cannot play', async ({ page }) => {
  await page.getByLabel('选择测试WAV').setInputFiles({ name: 'fixture.wav', mimeType: 'audio/wav', buffer: wave });
  await expect(page.getByRole('button', { name: '运行并播放' })).toBeEnabled();
  expect(await page.evaluate(() => (window as any).calls)).toBe(0);
  await page.getByRole('button', { name: '运行并播放' }).click();
  await expect(page.locator('.voice-phase')).toHaveText('正在生成回复');
  await page.getByRole('button', { name: '停止语音实验' }).click();
  await page.evaluate(() => (window as any).finishVoice());
  await expect(page.locator('.voice-phase')).toHaveText('已停止');
  await expect(page.getByText('测试回答', { exact: false })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).cancelled)).toBe(1);
  expect(await page.evaluate(() => (window as any).lastRequest.asrModel)).toBe('custom-asr');
  expect(await page.evaluate(() => (window as any).lastRequest.voiceBaseUrl)).toBe('https://example.com/v1');
  await page.getByRole('button', { name: '运行并播放' }).click();
  await page.evaluate(() => { HTMLMediaElement.prototype.play = () => Promise.reject(new Error('fixture playback denied')); (window as any).finishVoice(); });
  await expect(page.locator('.voice-phase')).toHaveText('未完成');
  await expect(page.locator('.voice-note')).toContainText('fixture playback denied');
  await expect(page.getByText('尚未开始播放')).toBeVisible();
});

test('completed synthesis replays locally without a second request, and prepared file name is echoed', async ({ page }) => {
  await page.getByLabel('选择测试WAV').setInputFiles({ name: 'fixture.wav', mimeType: 'audio/wav', buffer: wave });
  await expect(page.locator('.voice-note')).toContainText('音频已准备（fixture.wav）');
  await page.evaluate(() => {
    HTMLMediaElement.prototype.play = function () {
      const el = this;
      setTimeout(() => el.dispatchEvent(new Event('playing')), 0);
      setTimeout(() => el.dispatchEvent(new Event('ended')), 50);
      return Promise.resolve();
    };
  });
  await page.getByRole('button', { name: '运行并播放' }).click();
  await page.evaluate(() => (window as any).finishVoice());
  await expect(page.locator('.voice-phase')).toHaveText('播放已结束');
  await expect(page.getByText('测试文件（fixture.wav）')).toBeVisible();
  const firstLatency = await page.locator('tr', { hasText: '提交至播放开始' }).locator('td').textContent();
  expect(firstLatency).toMatch(/\d+ ms/);
  expect(await page.evaluate(() => (window as any).calls)).toBe(1);
  await page.getByRole('button', { name: '重播合成语音' }).click();
  await expect(page.locator('.voice-phase')).toHaveText('播放已结束');
  expect(await page.evaluate(() => (window as any).calls)).toBe(1);
  await expect(page.locator('tr', { hasText: '提交至播放开始' }).locator('td')).toHaveText(firstLatency!);
  expect(await page.evaluate(() => (window as any).cancelled)).toBe(0);
});

test('late microphone permission after cancellation releases tracks without recording or uploading', async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any;
    const context = new AudioContext();
    const destination = context.createMediaStreamDestination();
    w.testStream = destination.stream;
    navigator.mediaDevices.getUserMedia = () => new Promise(resolve => { w.grantMic = () => resolve(destination.stream); });
  });
  await page.getByRole('button', { name: '录制语音', exact: true }).click();
  await expect(page.locator('.voice-phase')).toHaveText('等待麦克风授权');
  await page.getByRole('button', { name: '停止语音实验' }).click();
  await page.evaluate(() => (window as any).grantMic());
  await expect.poll(() => page.evaluate(() => (window as any).testStream.getTracks()[0].readyState)).toBe('ended');
  expect(await page.evaluate(() => (window as any).calls)).toBe(0);
  await expect(page.locator('.voice-phase')).toHaveText('已停止');
});
