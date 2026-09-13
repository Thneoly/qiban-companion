import { expect, it } from 'vitest';
import { createPreviewClient } from './preview-client';

it('preview is explicitly nonpersistent, never executes tasks and cancels idempotently', async () => {
  const client = createPreviewClient();
  expect(await client.getRuntimeInfo()).toMatchObject({ runtime: 'preview', persistence: 'memory', executorAvailable: false });
  await expect(client.createTask(' ')).rejects.toThrow();
  const task = await client.createTask('写一份草稿');
  expect(task.status).toBe('queued');
  task.title = 'caller mutation';
  expect((await client.listTasks())[0]?.title).toBe('写一份草稿');
  expect((await client.cancelTask(task.id)).revision).toBe(1);
  expect((await client.cancelTask(task.id)).revision).toBe(1);
  expect(await createPreviewClient().listTasks()).toEqual([]);
});
