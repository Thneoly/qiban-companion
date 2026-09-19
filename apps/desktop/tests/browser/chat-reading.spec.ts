import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any;
    Object.defineProperty(window, 'isTauri', { value: true });
    const history = [{ user: '之前的问题', assistant: ('之前的回答，需要逐段阅读。\n').repeat(70) + 'x'.repeat(240) }];
    let serial = 0; const callbacks = new Map(); const listeners = new Map(); w.memoryEpoch=1;
    w.changeMemory = (notify = true) => { history.length=0; w.memoryEpoch++; if(notify) listeners.get("memory-changed")?.({payload:{contextEpoch:w.memoryEpoch,chatCleared:true}}); };
    w.requestCount = 0;
    w.failHistoryOnce = false;
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
      transformCallback: (cb: any) => { callbacks.set(++serial,cb); return serial; },
      unregisterCallback: () => {},
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { listeners.set(args.event,callbacks.get(args.handler)); return ++serial; }
        if (cmd === 'chat_context_epoch') return w.memoryEpoch;
        if (cmd === 'guide_status') return true;
        if (cmd === 'get_runtime_info') return { protocolVersion: 1, appVersion: 'test', runtime: 'desktop', persistence: 'sqlite', executorAvailable: false };
        if (cmd === 'chat_config') return { configured: true, model: 'reading-fixture', maxOutputTokens: 1024 };
        if (cmd === 'chat_history') { if (w.failHistoryOnce) { w.failHistoryOnce = false; throw Error('fixture history read failed'); } return history; }
        if (cmd === 'chat_clear') { if (w.failDelete) throw '删除本机对话失败，原记录仍保留'; history.length = 0; return; }
        if (cmd === 'chat_cancel') { w.streamFixture.fail(); return; }
        if (cmd === 'chat_generate') {
          w.requestCount++;
          return new Promise((resolve, reject) => {
            let reply = '';
            w.streamFixture = {
              part: (text: string) => { reply += text; args.onDelta.onmessage({ requestId: args.request.requestId, text }); },
              fail: () => reject('测试连接中断'),
              finish: () => { if (!w.failSave) history.push({ user: args.request.prompt, assistant: reply }); resolve({ requestId: args.request.requestId, elapsedMs: 400, historySaved: !w.failSave, usage: { total_tokens: 10 } }); }
            };
          });
        }
        return 1;
      }
    } });
  });
  await page.goto('/');
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '聊一聊', exact: true }).click();
  await page.getByRole('button', { name: '展开阅读', exact: true }).click();
});

test('restored history stays visible after failed deletion and clears only on success', async ({ page }) => {
  const reader = page.getByRole('region', { name: '对话阅读区' });
  await expect(page.getByRole('status')).toContainText('已恢复本机记录');
  await page.evaluate(() => { (window as any).failDelete = true; });
  await page.getByRole('button', { name: '清空对话', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('删除本机对话失败');
  await expect(reader).toContainText('之前的问题');
  await page.evaluate(() => { (window as any).failDelete = false; });
  await page.getByRole('button', { name: '清空对话', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('全部模型的对话记录已删除');
  await expect(reader.locator('.chat-turn')).toHaveCount(0);
  await page.getByRole('button', { name: '收起气泡' }).click();
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await expect(page.getByText('最近 0 轮', { exact: true })).toBeVisible();
});

test('a completed but unsaved reply is not presented as restored history', async ({ page }) => {
  await page.getByLabel('和栖栖说句话').fill('保存失败的问题');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await page.evaluate(() => { const w = window as any; w.failSave = true; w.streamFixture.part('完整但未保存的回答'); w.streamFixture.finish(); });
  await expect(page.locator('.chat-phase')).toHaveText('已完成');
  await expect(page.getByRole('status')).toContainText('未保存记录');
  await expect(page.getByRole('region', { name: '对话阅读区' })).toContainText('完整但未保存的回答');
  await expect(page.getByText('最近 1 轮', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '收起气泡' }).click();
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await expect(page.getByLabel('栖栖的回复')).not.toContainText('完整但未保存的回答');
});

test('reading fits pet window and new text respects manual scrolling', async ({ page }) => {
  const reader = page.getByRole('region', { name: '对话阅读区' });
  await expect(reader).toContainText('之前的问题');
  const bounds = await page.locator('.pet-dialog').boundingBox();
  expect(bounds!.height).toBeGreaterThan(350);
  const overflow = await reader.evaluate(e => e.scrollWidth - e.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await page.getByLabel('和栖栖说句话').fill('请继续');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(page.locator('.chat-phase')).toHaveText('等待回复');
  await reader.evaluate(e => { e.scrollTop = 0; e.dispatchEvent(new Event('scroll')); });
  await expect(page.getByRole('button', { name: '回到最新 ↓' })).toBeVisible();
  await page.evaluate(() => (window as any).streamFixture.part('新增回复\n'.repeat(40)));
  await expect(page.locator('.chat-phase')).toHaveText('正在回复');
  expect(await reader.evaluate(e => e.scrollTop)).toBe(0);
  await page.getByRole('button', { name: '回到最新 ↓' }).click();
  expect(await reader.evaluate(e => e.scrollHeight - e.clientHeight - e.scrollTop)).toBeLessThan(2);
  await page.evaluate(() => (window as any).streamFixture.finish());
  await expect(page.locator('.chat-phase')).toHaveText('已完成');
  await expect(reader.locator('.chat-turn')).toHaveCount(2);
  await expect(reader).toContainText('已加入前文');
  await page.screenshot({ path: test.info().outputPath('reading.png') });
  await page.getByRole('button', { name: '收回气泡', exact: true }).click();
  await expect(reader).toHaveCount(0);
  await expect(page.locator('.pet-dialog')).not.toHaveClass(/pet-dialog-reading/);
  expect((await page.locator('.pet-dialog').boundingBox())!.height).toBeLessThanOrEqual(224);
});

test('partial failure stays distinct from completed reply whose history refresh fails', async ({ page }) => {
  await page.getByLabel('和栖栖说句话').fill('重试的问题');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await page.evaluate(() => { (window as any).streamFixture.part('只有半句'); (window as any).streamFixture.fail(); });
  await expect(page.locator('.chat-phase')).toHaveText('未完成');
  await expect(page.locator('.pet-shell')).toHaveAttribute('data-companion-state', 'concerned');
  await expect(page.getByRole('region', { name: '对话阅读区' })).toContainText('未加入前文');
  await expect(page.getByLabel('和栖栖说句话')).toHaveValue('重试的问题');
  await expect(page.getByText('最近 1 轮', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await page.evaluate(() => { const w = window as any; w.streamFixture.part('完整的新回复'); w.failHistoryOnce = true; w.streamFixture.finish(); });
  await expect(page.locator('.chat-phase')).toHaveText('已完成');
  await expect(page.getByRole('status')).toContainText('回复已完成，但记录读取失败');
  await expect(page.locator('.pet-shell')).toHaveAttribute('data-companion-state', 'pleased');
  expect(await page.evaluate(() => (window as any).requestCount)).toBe(2);
  await page.getByRole('button', { name: '收起气泡', exact: true }).click();
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await expect(page.getByLabel('栖栖的回复')).toHaveText('完整的新回复');
  await expect(page.getByText('最近 2 轮', { exact: true })).toBeVisible();
});

test('companion follows real text phases and ignores late text after stopping or closing', async ({ page }) => {
  const shell = page.locator('.pet-shell');
  const portrait = page.locator('.pet-portrait svg');
  await expect(shell).toHaveAttribute('data-companion-state', 'attentive');
  await expect(portrait).toBeVisible();
  await page.getByLabel('和栖栖说句话').fill('陪我理清今天的想法');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(shell).toHaveAttribute('data-companion-state', 'thinking');
  await expect(portrait).toHaveAttribute('data-expression', 'thinking');
  await expect(page.locator('.pet-portrait .thought-dots')).toBeVisible();
  await page.evaluate(() => (window as any).streamFixture.part('我们可以从一件小事开始。'));
  await expect(shell).toHaveAttribute('data-companion-state', 'responding');
  await expect(page.locator('.pet-drag')).toContainText('正在回复你');
  await page.evaluate(() => (window as any).streamFixture.finish());
  await expect(portrait).toHaveAttribute('data-expression', 'pleased');
  await page.screenshot({ path: test.info().outputPath('companion-reading.png') });
  await page.getByRole('button', { name: '收起气泡', exact: true }).click();
  await expect(shell).toHaveAttribute('data-companion-state', 'idle');
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByLabel('和栖栖说句话').fill('先想一想');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(shell).toHaveAttribute('data-companion-state', 'thinking');
  await page.getByRole('button', { name: '停止', exact: true }).click();
  await page.evaluate(() => (window as any).streamFixture.part('迟到的内容'));
  await expect(shell).toHaveAttribute('data-companion-state', 'paused');
  await expect(page.getByLabel('栖栖的回复')).not.toContainText('迟到');
  await expect(page.getByRole('button', { name: '发送', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(shell).toHaveAttribute('data-companion-state', 'thinking');
  await page.getByRole('button', { name: '收起气泡', exact: true }).click();
  await page.evaluate(() => (window as any).streamFixture.part('收起后到达'));
  await expect(shell).toHaveAttribute('data-companion-state', 'idle');
  await page.getByRole('button', { name: '和栖栖互动' }).click();
  await page.getByRole('button', { name: '安静陪伴', exact: true }).click();
  await expect(shell).toHaveAttribute('data-companion-state', 'quiet');
  await expect(page.locator('.pet-character .resting-eyes')).toBeVisible();
  expect(await page.locator('.pet-character .creature').evaluate(e => getComputedStyle(e).animationName)).toBe('none');
});

test('reduced motion preserves companion feedback without animation', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByLabel('和栖栖说句话').fill('少一点动画');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(page.locator('.pet-shell')).toHaveAttribute('data-companion-state', 'thinking');
  expect(await page.locator('.pet-portrait .creature').evaluate(e => getComputedStyle(e).animationName)).toBe('none');
  expect(await page.locator('.pet-portrait .thought-dots').evaluate(e => getComputedStyle(e).animationName)).toBe('none');
  await expect(page.locator('.pet-presence')).toContainText('正在等回复');
});


test('memory deletion removes displayed text and ignores late stream callbacks', async ({ page }) => {
  await page.getByLabel('和栖栖说句话').fill('正在生成的问题');
  await page.getByRole('button', {name:'发送',exact:true}).click();
  await page.evaluate(() => { const w=window as any; w.streamFixture.part('即将删除的片段'); });
  await expect(page.getByRole('region',{name:'对话阅读区'})).toContainText('即将删除的片段');
  await page.evaluate(() => { const w=window as any; w.changeMemory(); w.streamFixture.part('删除后迟到的片段'); });
  await expect(page.getByRole('region',{name:'对话阅读区'})).not.toContainText('即将删除的片段');
  await expect(page.getByRole('region',{name:'对话阅读区'})).not.toContainText('删除后迟到的片段');
  await expect(page.getByRole('status')).toContainText('已刷新本机记录');
  await expect(page.getByText('最近 0 轮',{exact:true})).toBeVisible();
});

test('missed memory notification is reconciled before another request can be sent', async ({ page }) => {
  await page.evaluate(() => { (window as any).changeMemory(false); });
  await page.getByLabel('和栖栖说句话').fill('不应直接发送');
  await page.getByRole('button', {name:'发送',exact:true}).click();
  await expect(page.getByRole('status')).toContainText('已刷新本机记录');
  expect(await page.evaluate(()=>(window as any).requestCount)).toBe(0);
  await expect(page.getByRole('region',{name:'对话阅读区'})).not.toContainText('之前的问题');
});
