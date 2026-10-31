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

it('deletes exactly the named local row and refuses missing ids', async () => {
  const client = createPreviewClient();
  const gone = await client.createTask('取消后删除');
  const keep = await client.createTask('留下的');
  await client.cancelTask(gone.id);
  await client.deleteTask(gone.id);
  expect((await client.listTasks()).map(task => task.id)).toEqual([keep.id]);
  await expect(client.deleteTask(gone.id)).rejects.toThrow('未找到任务');
});
