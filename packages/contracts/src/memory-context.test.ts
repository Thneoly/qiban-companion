import { it, expect } from 'vitest';
import fixture from '../fixtures/manual-memory.json';
import { decodeContextPreview, decodeMemoryUsage, personalPolicyErrorMessage } from './memory-context';
import { decodeChatResult, decodeChatDelta, decodeRuntime } from './index';
const scope={baseUrl:'https://fixture.test',model:'m'};
const offlineFamily={status:'offline',policy:{enabled:false,revision:0,selectedIds:[]},items:[],inactiveSelectedIds:[],bodyChars:0,contextChars:0};
const preview={scope,contextEpoch:1,policy:{enabled:true,revision:1,selectedIds:[fixture.id]},items:[fixture],bodyChars:[...fixture.body].length,contextChars:500,personal:offlineFamily};
it('preview binds explicit ordered selection, size and scope',()=>{
  expect(decodeContextPreview(preview).items).toEqual([fixture]);
  for(const change of [{scope:null},{contextEpoch:NaN},{bodyChars:0},{contextChars:0},{policy:{...preview.policy,enabled:false}},{policy:{...preview.policy,selectedIds:[]}}]) expect(()=>decodeContextPreview({...preview,...change})).toThrow();
  // v3: the personal family is required — v2 payloads (no personal) fail.
  const {personal:_drop, ...v2} = preview as typeof preview & {personal:unknown};
  expect(()=>decodeContextPreview(v2)).toThrow();
});
const personalRecord={id:9,seq:9,type:'project' as const,project:'Game',title:'方向',content:'内容正文',importance:5,createdAt:'2026-09-26 03:41:02',updatedAt:'2026-09-26 03:41:02',validUntil:null,supersededBy:null,contradicts:null,tags:[],origin:'mcp'};
it('personal family accepts ordered subsets and rejects silent shrinkage',()=>{
  const online={status:'online',policy:{enabled:true,revision:1,selectedIds:[9,8,7]},items:[personalRecord,{...personalRecord,id:8,seq:8}],inactiveSelectedIds:[7],bodyChars:[...personalRecord.content].length*2,contextChars:300};
  const decoded=decodeContextPreview({...preview,personal:online});
  expect(decoded.personal.items.map(i=>i.id)).toEqual([9,8]);
  expect(decoded.personal.inactiveSelectedIds).toEqual([7]);
  // Order violation (8 before 9) is not the saved order.
  const reordered={...online,items:[{...personalRecord,id:8,seq:8},personalRecord]};
  expect(()=>decodeContextPreview({...preview,personal:reordered})).toThrow();
  // An item outside the selection, offline items, budget overflow, wrong
  // recomputed bodyChars, and offline-with-items are all protocol breaks.
  expect(()=>decodeContextPreview({...preview,personal:{...online,items:[personalRecord,{...personalRecord,id:99,seq:99}]}})).toThrow();
  expect(()=>decodeContextPreview({...preview,personal:{...online,items:[] , inactiveSelectedIds:[7]}})).toThrow();
  expect(()=>decodeContextPreview({...preview,personal:{...online,bodyChars:801}})).toThrow();
  expect(()=>decodeContextPreview({...preview,personal:{...online,status:'offline'}})).toThrow();
  expect(()=>decodeContextPreview({...preview,personal:{...online,policy:{...online.policy,enabled:false}}}));
});
it('usage receipt separates app and personal families',()=>{
  const usage={scope,contextEpoch:1,memories:[{id:fixture.id,revision:2}],bodyChars:10,contextChars:500,personal:{status:'sent',memories:[{id:9,seq:9}],bodyChars:10,contextChars:300}};
  expect(decodeMemoryUsage(usage)).toEqual(usage);
  const offline={...usage,personal:{status:'offline',memories:[],bodyChars:0,contextChars:0}};
  expect(decodeMemoryUsage(offline).personal.status).toBe('offline');
  for(const value of [undefined,{...usage,memories:[...usage.memories,...usage.memories]},{...usage,bodyChars:801},{...usage,contextEpoch:2**54},{...usage,personal:{status:'sent',memories:[{id:9,seq:0}]}},{...usage,personal:{status:'offline',memories:[{id:9,seq:9}]}}])expect(()=>decodeMemoryUsage(value)).toThrow();
  expect(()=>decodeChatDelta({requestId:'r',text:'text'})).toThrow();
  expect(()=>decodeChatResult({requestId:'r',elapsedMs:2,usage:null,historySaved:true})).toThrow();
  expect(()=>decodeRuntime({protocolVersion:2,appVersion:'x',runtime:'desktop',persistence:'sqlite',executorAvailable:false})).toThrow();
  expect(personalPolicyErrorMessage({code:'service_offline'})).toContain('未运行');
  expect(personalPolicyErrorMessage({code:'surprise'})).toContain('协议不兼容');
});
