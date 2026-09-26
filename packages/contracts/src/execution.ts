/** Additive local execution API; general inbox tasks remain non-executable. */
export const executionStatuses = ['waiting_confirmation','running','completed','cancelled','failed','unknown'] as const;
export type ExecutionStatus = typeof executionStatuses[number];
export interface ExecutionTask {
  id:string; actionId:string; sourceName:string; preview:string; artifactName:string; artifactHash:string;
  status:ExecutionStatus; revision:number; createdAt:number; updatedAt:number; note:string;
}
export interface ExecutionAttempt { id:string;actionId:string;owner:string;leaseUntil:number;status:ExecutionStatus }
export interface ExecutionEvent { sequence:number;taskId:string;revision:number;status:ExecutionStatus;createdAt:number }
export interface ExecutionDetail {task:ExecutionTask;attempts:ExecutionAttempt[];events:ExecutionEvent[]}
const object=(v:unknown):Record<string,unknown>=>{if(!v||typeof v!=='object'||Array.isArray(v))throw Error('执行响应格式不兼容');return v as Record<string,unknown>;};
const integer=(v:unknown)=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=0;
const uuid=(v:unknown)=>typeof v==='string'&&/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(v);
const status=(v:unknown)=>executionStatuses.includes(v as ExecutionStatus);
export function decodeExecutionTask(value:unknown):ExecutionTask {
  const v=object(value);
  if(!uuid(v.id)||!uuid(v.actionId)||typeof v.sourceName!=='string'||typeof v.preview!=='string'||v.preview.length>12000||
    v.artifactName!==`${v.actionId}.md`||typeof v.artifactHash!=='string'||!/^[a-f0-9]{64}$/.test(v.artifactHash)||
    !status(v.status)||!integer(v.revision)||!integer(v.createdAt)||!integer(v.updatedAt)||typeof v.note!=='string')throw Error('执行任务协议不兼容');
  return v as unknown as ExecutionTask;
}
export function decodeExecutions(value:unknown):ExecutionTask[] {
  if(!Array.isArray(value)||value.length>100)throw Error('执行列表协议不兼容');return value.map(decodeExecutionTask);
}
export function decodeExecutionDetail(value:unknown):ExecutionDetail {
  const v=object(value),task=decodeExecutionTask(v.task);
  if(!Array.isArray(v.attempts)||!Array.isArray(v.events))throw Error('执行记录协议不兼容');
  const attempts=v.attempts.map(value=>{const a=object(value);if(!uuid(a.id)||a.actionId!==task.actionId||typeof a.owner!=='string'||!integer(a.leaseUntil)||!status(a.status))throw Error('执行尝试协议不兼容');return a as unknown as ExecutionAttempt;});
  let previous=0;
  const events=v.events.map(value=>{const e=object(value);if(!integer(e.sequence)||(e.sequence as number)<=previous||e.taskId!==task.id||!integer(e.revision)||!status(e.status)||!integer(e.createdAt))throw Error('执行事件协议不兼容');previous=e.sequence as number;return e as unknown as ExecutionEvent;});
  if(!events.length||events.at(-1)!.revision!==task.revision||events.at(-1)!.status!==task.status)throw Error('执行快照版本不一致');
  return {task,attempts,events};
}
