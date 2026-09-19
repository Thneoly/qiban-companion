import {expect,test} from 'vitest';
import {decodeChatConfig,decodeChatDelta,decodeChatResult,decodeChatHistory,decodeModelSettings} from './index';
test('chat boundary rejects malformed events and preserves unknown usage',()=>{
  expect(decodeChatResult({requestId:'a',elapsedMs:5,historySaved:true,usage:null}).usage).toBeNull();
  expect(decodeChatResult({requestId:'a',elapsedMs:5,usage:null}).historySaved).toBe(false);
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:5,historySaved:'yes',usage:null})).toThrow();
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:-1,usage:null})).toThrow();
  expect(()=>decodeChatDelta({requestId:'a',text:42})).toThrow();
  expect(()=>decodeChatConfig({configured:'true',model:'glm-5.3'})).toThrow();
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:5,usage:{total_tokens:'secret'}})).toThrow();
});

test('history requires bounded complete text pairs',()=>{
  expect(decodeChatHistory([{user:'hello',assistant:'hi'}])).toHaveLength(1);
  expect(()=>decodeChatHistory([{user:'hello',assistant:42}])).toThrow();
  expect(()=>decodeChatHistory(Array(7).fill({user:'x',assistant:'y'}))).toThrow();
});

test('budget contracts reject missing, fractional and out-of-range values',()=>{
  for(const maxOutputTokens of [undefined,127,8193,1.5,'4096']) {
    expect(()=>decodeModelSettings({baseUrl:'https://example.com/v1',model:'test',useApiKey:false,hasApiKey:false,maxOutputTokens})).toThrow();
    expect(()=>decodeChatConfig({configured:true,model:'test',maxOutputTokens})).toThrow();
  }
  expect(decodeChatConfig({configured:true,model:'test',maxOutputTokens:4096}).maxOutputTokens).toBe(4096);
});
