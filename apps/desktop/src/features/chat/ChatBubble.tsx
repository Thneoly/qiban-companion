import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { decodeChatConfig, decodeChatDelta, decodeChatResult } from '@companion/contracts';

export function ChatBubble({ onThinking }: { onThinking: (active: boolean) => void }) {
  const [configured,setConfigured]=useState(false);
  const [model,setModel]=useState('glm-5.3');
  const [draft,setDraft]=useState('');
  const [reply,setReply]=useState('');
  const [status,setStatus]=useState(nativeDesktop?'正在检查模型配置…':'浏览器预览不调用模型，请运行桌面版');
  const [busy,setBusy]=useState(false);
  const current=useRef<string|null>(null);
  const alive=useRef(true);
  useEffect(()=>{
    alive.current=true;
    if(nativeDesktop) void invoke('chat_config').then(value=>{
      if(!alive.current)return;
      const config=decodeChatConfig(value);setConfigured(config.configured);setModel(config.model);
      setStatus(config.configured?'单轮对话 · 发送至已配置的模型服务':'尚未配置密钥，请打开模型设置');
    }).catch(()=>{if(alive.current)setStatus('读取模型配置失败');});
    return ()=>{alive.current=false;const id=current.current;current.current=null;if(id)void invoke('chat_cancel',{requestId:id}).catch(()=>{});onThinking(false);};
  },[onThinking]);
  async function submit(event:FormEvent) {
    event.preventDefault();if(busy||!configured||!draft.trim())return;
    const id=crypto.randomUUID();current.current=id;setBusy(true);setReply('');setStatus('正在生成…');onThinking(true);
    const channel=new Channel<unknown>();
    channel.onmessage=value=>{
      if(!alive.current||current.current!==id)return;
      try {const delta=decodeChatDelta(value);if(delta.requestId===id)setReply(text=>text+delta.text);}
      catch {current.current=null;setStatus('回复协议不兼容');void invoke('chat_cancel',{requestId:id}).catch(()=>{});}
    };
    try {
      const result=decodeChatResult(await invoke('chat_generate',{request:{requestId:id,prompt:draft.trim()},onDelta:channel}));
      if(alive.current&&current.current===id&&result.requestId===id)
        setStatus('回复结束 · '+(result.usage?.total_tokens==null?'用量未返回':result.usage.total_tokens+' tokens'));
    } catch(error) {
      if(alive.current&&current.current===id)setStatus(typeof error==='string'?error:'请求失败，请重试');
    } finally {
      if(alive.current){setBusy(false);onThinking(false);}
      if(current.current===id)current.current=null;
    }
  }
  async function stop() {
    const id=current.current;if(!id)return;
    current.current=null;setStatus('正在停止…');
    try {await invoke('chat_cancel',{requestId:id});if(alive.current)setStatus('已停止接收回复 · 已产生的用量可能计费');}
    catch {if(alive.current)setStatus('停止请求失败，已屏蔽旧回复');}
  }
  return <div className="chat-bubble">
    <div className="chat-output" aria-label="栖栖的回复" aria-live="polite">{reply||'想聊点什么？'}</div>
    <form onSubmit={submit}>
      <label className="sr-only" htmlFor="chat-draft">和栖栖说句话</label>
      <input id="chat-draft" maxLength={2000} value={draft} onChange={e=>setDraft(e.target.value)} placeholder="和我说说…" disabled={busy}/>
      {busy?<button type="button" className="pet-save" onClick={()=>void stop()}>停止</button>:<button className="pet-save" disabled={!configured||!draft.trim()}>发送</button>}
    </form>
    <p className="chat-status" role="status">{status}</p>
    <small className="chat-model">{model} · 收起停止对话 · 不保存会话</small>
  </div>;
}
