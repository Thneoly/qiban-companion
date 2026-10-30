import { test, expect } from '@playwright/test';

test('browser preview reports its limits and records/cancels/deletes a task without claiming execution', async ({ page }) => {
  const errors: string[] = [];
  const confirms: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('dialog', async dialog => { confirms.push(dialog.message()); await dialog.accept(); });
  await page.goto('/?view=panel');
  await expect(page.getByText('浏览器预览', { exact: true })).toBeVisible();
  await expect(page.getByText('浏览器刷新后，待办会清空')).toBeVisible();
  await page.getByLabel('任务标题', { exact: true }).fill('整理一份周末阅读清单');
  await page.getByRole('button', { name: '记下来' }).click();
  await expect(page.getByText('待执行', { exact: true })).toBeVisible();
  // 终态默认折叠：取消后不再渲染整行，只剩一条计数行。
  await page.getByRole('button', { name: '取消任务：整理一份周末阅读清单' }).click();
  await expect(page.getByRole('button', { name: /已结束的 1 条/ })).toBeVisible();
  await expect(page.getByText('已取消', { exact: true })).toBeHidden();
  // 点开计数行展开；删除先确认，文案讲清“仅从列表移除”的边界。
  await page.getByRole('button', { name: /已结束的 1 条/ }).click();
  await expect(page.getByText('已取消', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '删除任务：整理一份周末阅读清单' }).click();
  expect(confirms[0]).toContain('仅从这台设备的待办列表移除');
  expect(confirms[0]).toContain('任务事实沉淀与聊天记录不受影响');
  await expect(page.getByRole('button', { name: /已结束的/ })).toBeHidden();
  await page.getByRole('button', { name: '和栖栖打个招呼' }).click();
  await expect(page.getByText('收到你的招呼啦。')).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('desktop-preview.png'), fullPage: true });
  await page.reload();
  await expect(page.getByText('留一点空间，给下一个好想法')).toBeVisible();
  expect(errors).toEqual([]);
});

test('status filter offers three quick toggles, todo+doing by default', async ({ page }) => {
  await page.goto('/?view=panel');
  await page.getByLabel('任务标题', { exact: true }).fill('筛选里的第一条');
  await page.getByRole('button', { name: '记下来' }).click();
  await expect(page.getByText('筛选里的第一条', { exact: true })).toBeVisible();
  // 默认「待办+进行中」在选；无终态时「已结束」不可点。
  await expect(page.getByRole('button', { name: /待办 1/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: /进行中 0/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: /已结束 0/ })).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('button', { name: /已结束 0/ })).toBeDisabled();
  // 关掉「待办」：条目隐藏并提示当前筛选为空；点回恢复。
  await page.getByRole('button', { name: /待办 1/ }).click();
  await expect(page.getByText('筛选里的第一条', { exact: true })).toBeHidden();
  await expect(page.getByText('当前筛选下没有显示中的待办。', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: /待办 1/ }).click();
  await expect(page.getByText('筛选里的第一条', { exact: true })).toBeVisible();
});

test('narrow preview remains usable', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?view=panel');
  await expect(page.getByRole('heading', { name: '今天，也一起慢慢来。' })).toBeVisible();
  await expect(page.getByLabel('任务标题', { exact: true })).toBeEnabled();
  const hasOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(hasOverflow).toBe(false);
});
