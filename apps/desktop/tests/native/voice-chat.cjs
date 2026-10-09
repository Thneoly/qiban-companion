// Windows acceptance: the pet-window voice turn over real IPC (CDP), with a
// synthetic capture device and localhost-only fixtures. Never touches real
// microphone hardware, provider accounts or production profiles.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),http=require('node:http');
const {spawn}=require('node:child_process');
const {DatabaseSync}=require('node:sqlite'); const {chromium,expect}=require('@playwright/test');
const exe=process.env.QIBAN_ACCEPTANCE_EXE,id=process.env.QIBAN_ACCEPTANCE_ID;
assert(exe&&fs.existsSync(exe));assert(/^dev\.qiban\.companion\.acceptance\.m2-[a-z0-9-]+$/.test(id||''));
const directory=path.join(process.env.LOCALAPPDATA,id); assert(!fs.existsSync(directory),'Refusing an existing profile');
const evidence=path.resolve('.cache',id+'-'+require('node:crypto').randomUUID());fs.mkdirSync(evidence,{recursive:true});
const port=Number(process.env.QIBAN_CDP_PORT||9442),delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser,pet,panel;const requests=[],ipcCalls=[];let heldChat,heldSpeech,chatRequests=0,fixtureFailure=null;const speechInputs=[],closes=[];
/** Rethrows the first fixture-side protocol violation at the next main-flow
 * checkpoint; asserts inside the http handler must not crash the harness. */
const fixtures=()=>{if(fixtureFailure)throw fixtureFailure;};
const wave=()=>{ // PCM16 mono 16kHz silence: passes wav_seconds, never matches the tone layout.
  const data=Buffer.alloc(1600*2);const out=Buffer.alloc(44+data.length);
  out.write('RIFF',0);out.writeUInt32LE(out.length-8,4);out.write('WAVE',8);out.write('fmt ',12);out.writeUInt32LE(16,16);
  out.writeUInt16LE(1,20);out.writeUInt16LE(1,22);out.writeUInt32LE(16000,24);out.writeUInt32LE(32000,28);out.writeUInt16LE(2,32);out.writeUInt16LE(16,34);
  out.write('data',36);out.writeUInt32LE(data.length,40);data.copy(out,44);return out;};
const fixtureWav=wave();
const server=http.createServer((req,res)=>{const chunks=[];req.on('data',c=>chunks.push(c));req.on('end',()=>{const body=Buffer.concat(chunks);
  try{
  assert(!req.headers.authorization,'fixtures must run without credentials');
  requests.push({path:req.url,bytes:body.length});
  if(req.url.endsWith('/audio/transcriptions')){
    const raw=body.toString('latin1');
    assert(raw.includes('native-asr')&&raw.includes('recording.wav'),'asr multipart shape');
    res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({text:'本机语音测试'}));return;}
  if(req.url.endsWith('/audio/speech')){
    const input=JSON.parse(body.toString('utf8')).input;
    speechInputs.push(input);
    if(input==='你好呀。'){heldSpeech=res;res.on('error',()=>{});res.on('close',()=>closes.push('speech'));return;} // held: the interrupt case speaks this sentence
    res.writeHead(200,{'Content-Type':'audio/wav','Content-Length':fixtureWav.length});res.end(fixtureWav);return;}
  if(req.url.endsWith('/chat/completions')){
    chatRequests++;res.writeHead(200,{'Content-Type':'text/event-stream'});
    if(chatRequests===1){res.write('data: '+JSON.stringify({choices:[{delta:{content:'你好呀。'}}]})+'\n\n');heldChat=res;res.on('error',()=>{});res.on('close',()=>closes.push('chat'));return;}
    res.write('data: '+JSON.stringify({choices:[{delta:{content:'第二句。'}}]})+'\n\n');
    res.write('data: '+JSON.stringify({choices:[{delta:{content:'结束了。'}}]})+'\n\n');
    res.write('data: '+JSON.stringify({choices:[{finish_reason:'stop'}],usage:{total_tokens:7}})+'\n\n');
    res.end('data: [DONE]\n\n');return;}
  assert.fail('unexpected fixture request '+req.url);
  }catch(error){fixtureFailure ??= error;res.destroy();}});});
server.on('clientError',(_error,socket)=>socket.end());
async function start(){
  browser=undefined;
  child=spawn(exe,[],{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,WEBVIEW2_USER_DATA_FOLDER:path.join(directory,'test-webview')}});
  for(let n=0;n<100;n++){try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);break;}catch{await delay(100);}}
  assert(browser);pet=panel=undefined;
  for(let n=0;n<100;n++){const pages=browser.contexts().flatMap(c=>c.pages());pet=pages.find(p=>p.url().includes('tauri')&&!p.url().includes('view=panel'));panel=pages.find(p=>p.url().includes('view=panel'));if(pet&&panel)break;await delay(100);}
  assert(pet&&panel);await pet.getByRole('button',{name:'和栖栖互动'}).waitFor();
  assert(fs.existsSync(path.join(directory,'instance.lock')),'Wrong build profile; refusing mutations');
  const db=new DatabaseSync(path.join(directory,'chat-history.db'),{readOnly:true});assert.equal(db.prepare('PRAGMA user_version').get().user_version,4);db.close();
  // The invoke tap that works on the real machine: WebView2 exposes every
  // tauri invoke as an http://ipc.localhost/<command> fetch with the args as
  // the POST body. window.__TAURI_INTERNALS__ and window.ipc are injected
  // non-writable, so in-page monkey-patching silently does nothing.
  pet.on('request',request=>{
    const match=request.url().match(/^http:\/\/ipc\.localhost\/([^/?]+)/);
    if(!match)return;
    let args;try{args=request.postData()?JSON.parse(request.postData()):{};}catch{args={};}
    ipcCalls.push({command:decodeURIComponent(match[1]),args});
  });
}
async function stop(){if(browser)await browser.close();browser=undefined;if(child){const process=child;child=undefined;const exited=new Promise(r=>process.once('exit',r));process.kill();await exited;}await delay(400);}
const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
(async()=>{try{
  let occupied=false;try{await fetch(`http://127.0.0.1:${port}/json/version`);occupied=true;}catch{} assert(!occupied);
  await new Promise(r=>server.listen(0,'127.0.0.1',r));
  const chatBase=`http://127.0.0.1:${server.address().port}`,voiceBase=chatBase+'/voice';
  await start();
  // First assertion: the pet window itself exposes the microphone surface.
  const surface=await pet.evaluate(async()=>{const out={gum:typeof navigator.mediaDevices?.getUserMedia};
    try{out.permission=(await navigator.permissions.query({name:'microphone'})).state;}catch{out.permission='unavailable';}
    return out;});
  assert(surface.gum==='function','pet window has no getUserMedia');
  assert(surface.permission!=='denied','pet window microphone permission denied');
  // Synthetic capture device (no hardware), real MediaRecorder pipeline, a
  // play() wrapper that cannot wedge on autoplay, and blob accounting. The
  // invoke side is observed from CDP (ipc.localhost requests), not patched.
  await pet.evaluate(()=>{
    const w=window;w.testPlays=[];w.testBlob={created:0,revoked:0};w.testMic={tracks:[]};
    const realCreate=URL.createObjectURL,realRevoke=URL.revokeObjectURL;
    URL.createObjectURL=value=>{w.testBlob.created++;return realCreate.call(URL,value);};
    URL.revokeObjectURL=value=>{w.testBlob.revoked++;return realRevoke.call(URL,value);};
    // The capture graph is built lazily inside getUserMedia, i.e. behind the
    // user gesture that clicked the mic, so autoplay policy cannot leave a
    // suspended context producing unplayable frames.
    let audio=null,gain=null;
    const ensureGraph=()=>{
      if(audio)return;
      audio=new AudioContext();w.testAudio=audio;void audio.resume().catch(()=>{});
      const oscillator=audio.createOscillator();oscillator.frequency.value=220;
      gain=audio.createGain();gain.gain.value=0.1;oscillator.connect(gain);oscillator.start();};
    Object.defineProperty(navigator.mediaDevices,'getUserMedia',{value:async()=>{
      ensureGraph();
      const destination=audio.createMediaStreamDestination();gain.connect(destination);
      w.testMic.tracks.push(...destination.stream.getTracks());return destination.stream;}});
    const realPlay=HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play=function(){const element=this;w.testPlays.push(element.src);
      realPlay.call(element)?.catch?.(()=>{}); // autoplay denial must not wedge the queue
      setTimeout(()=>{try{element.pause();}catch{}element.dispatchEvent(new Event('ended'));},150);
      return Promise.resolve();};});
  await invoke(panel,'model_settings_save',{config:{baseUrl:chatBase,model:'native-chat',useApiKey:false,maxOutputTokens:1024}});
  await invoke(panel,'voice_settings_save',{config:{voiceBaseUrl:voiceBase,useVoiceKey:false,asrModel:'native-asr',ttsModel:'native-tts',voice:'native-voice'}});
  // Raw IPC claim: voice_speak returns tauri::ipc::Response, which the webview
  // resolves as an ArrayBuffer, byte-exact through the first-clip passthrough.
  const probe=await pet.evaluate(async base=>{const native=window.__TAURI_INTERNALS__;const progress=[];
    const callback=native.transformCallback(raw=>{if(raw.message)progress.push(raw.message);});
    try{const value=await native.invoke('voice_speak',{request:{requestId:'native-raw-probe',turnId:'native-raw-turn',expectedBaseUrl:base,seq:0,text:'直连探针句',first:true},onProgress:`__CHANNEL__:${callback}`});
      const bytes=value instanceof ArrayBuffer?new Uint8Array(value):null;
      return {ok:true,rawType:value?.constructor?.name,byteLength:bytes?bytes.byteLength:Array.isArray(value)?value.length:null,head:bytes?[...bytes.slice(0,8)]:null,progress};}
    catch(error){return {ok:false,error:String(error)};}},voiceBase);
  assert(probe.ok,`raw probe failed: ${probe.error}`);
  assert(probe.rawType==='ArrayBuffer',`raw ipc type was ${probe.rawType}`);
  assert(probe.byteLength===fixtureWav.length&&JSON.stringify(probe.head)===JSON.stringify([...fixtureWav.slice(0,8)]),'raw wav bytes altered');
  assert.deepStrictEqual(probe.progress,[{requestId:'native-raw-probe',trimmed:null}],'progress channel payload shape');
  await invoke(pet,'voice_turn_cancel',{turnId:'native-raw-turn'});
  // Turn A: record, auto-send, then interrupt while chat and tts are in flight.
  // A fresh profile may surface the first-use guide depending on when the pet
  // window's guide_status read lands - handle either deterministically.
  await pet.getByRole('button',{name:'和栖栖互动'}).click();
  const chatMode=pet.getByRole('button',{name:'聊一聊',exact:true});
  const firstUse=pet.getByLabel('初次使用指南');
  await expect(chatMode.or(firstUse)).toBeVisible({timeout:15000});
  if(await firstUse.isVisible())await pet.getByRole('button',{name:'知道了，开始相处'}).click();
  await chatMode.click();
  const mic=pet.locator('button.pet-voice');
  await expect(mic).toHaveText('语音说话',{timeout:15000}); // pet ACL + settings read on the real machine
  await mic.click();
  await expect(mic).toHaveText('结束录音');
  await delay(700);
  await mic.click(); // stop -> transcribe -> auto-send
  await expect(pet.getByText('我听到：本机语音测试')).toBeVisible({timeout:15000});
  await expect(mic).toHaveText('停止朗读',{timeout:15000});
  // Wait for THIS turn's first clip specifically - the raw probe's own
  // speech request must not satisfy the wait.
  for(let n=0;n<100&&!speechInputs.includes('你好呀。');n++)await delay(100);
  fixtures();
  assert(speechInputs.includes('你好呀。'),"the turn's first sentence never reached /audio/speech");
  await pet.screenshot({path:path.join(evidence,'voice-turn-speaking.png')});
  await mic.click(); // 停止朗读
  await expect(mic).toHaveText('语音说话');
  assert(await pet.evaluate(()=>window.testPlays.length)===0,'interrupted turn must not have played');
  // The cancellations must reach the transport: dropping the futures on the
  // rust side aborts both in-flight provider connections server-side.
  for(let n=0;n<50&&!(closes.includes('speech')&&closes.includes('chat'));n++)await delay(100);
  assert(closes.includes('speech')&&closes.includes('chat'),'cancel did not abort the in-flight tts/chat connections');
  // The wire must show exactly one chat_cancel, tied to this turn's request.
  const speakCall=ipcCalls.find(c=>c.command==='voice_speak'&&c.args.request?.turnId&&c.args.request.turnId!=='native-raw-turn');
  assert(speakCall,'the turn never issued a voice_speak over ipc');
  const turn=speakCall.args.request.turnId;
  const chatCancels=ipcCalls.filter(c=>c.command==='chat_cancel');
  assert(chatCancels.length===1,'chat_cancel not issued with the interrupt');
  const chatRequestIds=ipcCalls.filter(c=>c.command==='chat_generate').map(c=>c.args.request?.requestId);
  assert(chatCancels[0].args.requestId&&chatRequestIds.includes(chatCancels[0].args.requestId),'chat_cancel does not match the turn chat request');
  // The probe cleanup cancelled 'native-raw-turn' first; the interrupt must
  // cancel exactly this turn.
  assert.deepStrictEqual(ipcCalls.filter(c=>c.command==='voice_turn_cancel').map(c=>c.args.turnId),['native-raw-turn',turn],'unexpected voice_turn_cancel sequence');
  assert.deepEqual(await invoke(pet,'chat_history'),[],'interrupted turn must not enter history');
  // The held responses land late: neither the clip nor the late chat text may
  // surface. The sockets are already aborted, so the writes go nowhere - which
  // is itself the point - and must not crash the fixture.
  try{heldSpeech.writeHead(200,{'Content-Type':'audio/wav'});heldSpeech.end(fixtureWav);}catch{}
  try{heldChat.end('data: '+JSON.stringify({choices:[{delta:{content:'迟到文本'}}]})+'\n\n'+'data: '+JSON.stringify({choices:[{finish_reason:'stop'}],usage:{total_tokens:5}})+'\n\n'+'data: [DONE]\n\n');}catch{}
  await delay(300);
  assert(await pet.evaluate(()=>window.testPlays.length)===0,'late clip played after interrupt');
  assert.deepEqual(await invoke(pet,'chat_history'),[],'late chat text entered history');
  // Turn B: a fresh voice turn completes end to end with queued clips.
  await mic.click();
  await expect(mic).toHaveText('结束录音');
  await delay(700);
  await mic.click();
  for(let n=0;n<150&&await pet.evaluate(()=>window.testPlays.length)<2;n++)await delay(100);
  fixtures();
  await expect(mic).toHaveText('语音说话',{timeout:15000});
  assert(await pet.evaluate(()=>window.testPlays.length)===2,'second turn should play two clips');
  assert.deepEqual(await invoke(pet,'chat_history'),[{user:'本机语音测试',assistant:'第二句。结束了。'}]);
  // Device release and blob hygiene across both turns.
  const released=await pet.evaluate(()=>({tracks:window.testMic.tracks.map(t=>t.readyState),blob:{...window.testBlob},plays:window.testPlays.length}));
  assert(released.tracks.length===2&&released.tracks.every(state=>state==='ended'),'capture tracks not released');
  assert(released.blob.created===released.blob.revoked,'object urls leaked');
  await pet.evaluate(()=>{try{void window.testAudio.close();}catch{}});
  await pet.screenshot({path:path.join(evidence,'voice-turn-complete.png')});
  await stop();
  fixtures();
  fs.writeFileSync(path.join(evidence,'requests.json'),JSON.stringify(requests,null,2));
  const result={passed:true,identifier:id,micPermission:surface.permission,rawIpc:probe.rawType,requests:requests.length,
    cases:['pet mic surface available','raw wav ipc ArrayBuffer byte-exact','voice turn auto-send over the chat chain',
      'interrupt aborts in-flight chat and tts at the transport','late clip and late text never surface',
      'second turn completes with queued clips','tracks released and no blob leaks','history keeps only the completed turn']};
  fs.writeFileSync(path.join(evidence,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{
  // Evidence lands on disk even when an assert fails mid-flow.
  try{fs.writeFileSync(path.join(evidence,'wire.json'),JSON.stringify({ipcCalls,requests,speechInputs,closes},null,2));}catch{}
  console.error('evidence: '+evidence);
  await stop();server.closeAllConnections();server.close();}})().catch(error=>{console.error(error);process.exitCode=1;});
