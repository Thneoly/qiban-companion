import {expect,test} from 'vitest';
import {decodeChatConfig,decodeChatDelta,decodeChatResult,decodeChatHistory,decodeModelSettings,decodeModelTestOutcome,decodeVoiceSettings,decodeVoiceTranscribe,decodeVoiceSpeakProgress} from './index';
const memoryUsage={scope:{baseUrl:'https://fixture.test',model:'m'},contextEpoch:0,memories:[],bodyChars:0,contextChars:0,personal:{status:'sent',memories:[],bodyChars:0,contextChars:0}};
test('chat boundary rejects malformed events and preserves unknown usage',()=>{
  expect(decodeChatResult({requestId:'a',elapsedMs:5,memoryUsage,historySaved:true,usage:null}).usage).toBeNull();
  expect(decodeChatResult({requestId:'a',elapsedMs:5,memoryUsage,usage:null}).historySaved).toBe(false);
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:5,memoryUsage,historySaved:'yes',usage:null})).toThrow();
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:-1,usage:null})).toThrow();
  expect(()=>decodeChatDelta({requestId:'a',text:42})).toThrow();
  expect(()=>decodeChatConfig({configured:'true',model:'glm-5.3'})).toThrow();
  expect(()=>decodeChatResult({requestId:'a',elapsedMs:5,memoryUsage,usage:{total_tokens:'secret'}})).toThrow();
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

test('voice settings decode saved labs config and reject missing or oversized fields',()=>{
  const saved={voiceBaseUrl:'https://open.bigmodel.cn/api/paas/v4',useVoiceKey:true,asrModel:'glm-asr-2512',ttsModel:'glm-tts',voice:'tongtong',hasVoiceKey:true};
  expect(decodeVoiceSettings(saved)).toEqual(saved);
  for(const patch of [{useVoiceKey:'yes'},{asrModel:undefined},{hasVoiceKey:undefined},{hasVoiceKey:'secret'},{voiceBaseUrl:'x'.repeat(513)},{voice:'y'.repeat(161)}])
    expect(()=>decodeVoiceSettings({...saved,...patch})).toThrow();
});

test('model test outcome decode accepts honest verdicts and rejects malformed payloads',()=>{
  expect(decodeModelTestOutcome({ok:true,detail:'连接成功：服务接受了地址、密钥与模型编码。'})).toEqual({ok:true,detail:'连接成功：服务接受了地址、密钥与模型编码。'});
  expect(decodeModelTestOutcome({ok:false,detail:'无法连接服务地址：网络不通，或地址填写有误'}).ok).toBe(false);
  for(const patch of [{ok:'yes'},{ok:undefined},{detail:''},{detail:undefined},{detail:'x'.repeat(401)}])
    expect(()=>decodeModelTestOutcome({ok:true,detail:'连接成功',...patch})).toThrow();
});

test('voice transcribe decode keeps bounded text and rejects empty or malformed payloads',()=>{
  expect(decodeVoiceTranscribe({requestId:'turn-1',transcript:'你好栖栖',recognitionMs:314}).transcript).toBe('你好栖栖');
  for(const patch of [{transcript:''},{transcript:'x'.repeat(1801)},{transcript:42},{requestId:''},{recognitionMs:-1},{recognitionMs:1.5}])
    expect(()=>decodeVoiceTranscribe({requestId:'turn-1',transcript:'你好栖栖',recognitionMs:314,...patch})).toThrow();
});

test('voice speak progress decode takes the trim flag plus the tone-detected verdict',()=>{
  expect(decodeVoiceSpeakProgress({requestId:'turn-1',trimmed:true,toneDetected:true}).trimmed).toBe(true);
  expect(decodeVoiceSpeakProgress({requestId:'turn-1',trimmed:false,toneDetected:true}).toneDetected).toBe(true);
  expect(decodeVoiceSpeakProgress({requestId:'turn-1',trimmed:false,toneDetected:false}).toneDetected).toBe(false);
  // The old null variant meant "first clip, tone kept on purpose" — that
  // exemption is gone, so null must now be rejected. toneDetected is equally
  // required: a tone-free provider reports trimmed=false on every clip, and
  // without the distinction that reads as a trim failure (TUWE-6).
  for(const patch of [{trimmed:'yes'},{trimmed:undefined},{trimmed:null},{toneDetected:undefined},{toneDetected:null},{toneDetected:'yes'},{requestId:''}])
    expect(()=>decodeVoiceSpeakProgress({requestId:'turn-1',trimmed:true,toneDetected:true,...patch})).toThrow();
});
