import { test, expect } from '@playwright/test';

test('browser preview reports its limits and records/cancels a task without claiming execution', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/?view=panel');
  await expect(page.getByText('浏览器预览', { exact: true })).toBeVisible();
  await expect(page.getByText('浏览器刷新后，待办会清空')).toBeVisible();
  await page.getByLabel('任务标题', { exact: true }).fill('整理一份周末阅读清单');
  await page.getByRole('button', { name: '记下来' }).click();
  await expect(page.getByText('整理一份周末阅读清单', { exact: true })).toBeVisible();
  await expect(page.getByText('待执行', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '取消任务：整理一份周末阅读清单' }).click();
  await expect(page.getByText('已取消', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '和栖栖打个招呼' }).click();
  await expect(page.getByText('收到你的招呼啦。')).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('desktop-preview.png'), fullPage: true });
  await page.reload();
  await expect(page.getByText('留一点空间，给下一个好想法')).toBeVisible();
  expect(errors).toEqual([]);
});

test('narrow preview remains usable', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?view=panel');
  await expect(page.getByRole('heading', { name: '今天，也一起慢慢来。' })).toBeVisible();
  await expect(page.getByLabel('任务标题', { exact: true })).toBeEnabled();
  const hasOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(hasOverflow).toBe(false);
});
