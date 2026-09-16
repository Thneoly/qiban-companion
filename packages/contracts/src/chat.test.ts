import {expect,test} from 'vitest';
import {decodeChatConfig,decodeChatDelta,decodeChatResult,decodeChatHistory} from './index';
test('chat boundary rejects malformed events and preserves unknown usage',()=>{
  expect(decodeChatResult({requestId:'a',elapsedMs:5,usage:null}).usage).toBeNull();
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
