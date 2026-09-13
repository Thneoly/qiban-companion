import { invoke, isTauri } from '@tauri-apps/api/core';
import { decodeRuntime, decodeTask, decodeTasks, validateTitle, type CompanionClient } from '@companion/contracts';
import { createPreviewClient } from './preview-client';

function createNativeClient(): CompanionClient {
  return {
    getRuntimeInfo: async () => decodeRuntime(await invoke('get_runtime_info')),
    listTasks: async () => decodeTasks(await invoke('list_tasks')),
    createTask: async title => decodeTask(await invoke('create_task', { title: validateTitle(title) })),
    cancelTask: async id => decodeTask(await invoke('cancel_task', { id })),
  };
}
export const client = isTauri() ? createNativeClient() : createPreviewClient();
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object' && 'message' in error && typeof error.message === 'string') return error.message;
  return '操作未完成，请重试。';
}
