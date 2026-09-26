import {expect,it} from 'vitest';
import {decodeExecutionDetail,decodeExecutionTask} from './execution';
const id='00000000-0000-4000-8000-000000000001',actionId='00000000-0000-4000-8000-000000000002';
const task={id,actionId,sourceName:'demo.txt',preview:'摘录',artifactName:`${actionId}.md`,artifactHash:'a'.repeat(64),status:'waiting_confirmation',revision:0,createdAt:1,updatedAt:1,note:'等待确认'};
it('rejects incompatible states, outside paths and unsafe counters',()=>{
  expect(decodeExecutionTask(task).id).toBe(id);
  for(const change of [{status:'done'},{artifactName:'../outside.md'},{revision:Number.MAX_SAFE_INTEGER+1},{artifactHash:'unverified'}])expect(()=>decodeExecutionTask({...task,...change})).toThrow();
});
it('rejects stale snapshots and events for another task',()=>{
  const event={sequence:1,taskId:id,revision:0,status:'waiting_confirmation',createdAt:1};
  expect(decodeExecutionDetail({task,attempts:[],events:[event]}).events).toHaveLength(1);
  for(const events of [[],[event,event],[{...event,revision:1}],[{...event,taskId:actionId}]])expect(()=>decodeExecutionDetail({task,attempts:[],events})).toThrow();
});
