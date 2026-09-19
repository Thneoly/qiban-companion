import { it, expect } from 'vitest';
import fixture from '../fixtures/manual-memory.json';
import { decodeContextPreview, decodeMemoryUsage } from './memory-context';
import { decodeChatResult, decodeChatDelta, decodeRuntime } from './index';
const scope={baseUrl:'https://fixture.test',model:'m'};
const preview={scope,contextEpoch:1,policy:{enabled:true,revision:1,selectedIds:[fixture.id]},items:[fixture],bodyChars:[...fixture.body].length,contextChars:500};
it('preview binds explicit ordered selection, size and scope',()=>{
  expect(decodeContextPreview(preview).items).toEqual([fixture]);
  for(const change of [{scope:null},{contextEpoch:NaN},{bodyChars:0},{contextChars:0},{policy:{...preview.policy,enabled:false}},{policy:{...preview.policy,selectedIds:[]}}]) expect(()=>decodeContextPreview({...preview,...change})).toThrow();
});
it('v2 requires a valid submitted snapshot and rejects old protocol',()=>{
  const usage={scope,contextEpoch:1,memories:[{id:fixture.id,revision:2}],bodyChars:10,contextChars:500};
  expect(decodeMemoryUsage(usage)).toEqual(usage);
  for(const value of [undefined,{...usage,memories:[...usage.memories,...usage.memories]},{...usage,bodyChars:801},{...usage,contextEpoch:2**54}])expect(()=>decodeMemoryUsage(value)).toThrow();
  expect(()=>decodeChatDelta({requestId:'r',text:'text'})).toThrow();
  expect(()=>decodeChatResult({requestId:'r',elapsedMs:2,usage:null,historySaved:true})).toThrow();
  expect(()=>decodeRuntime({protocolVersion:1,appVersion:'x',runtime:'desktop',persistence:'sqlite',executorAvailable:false})).toThrow();
});
