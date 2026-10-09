import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { decodeContextPreview, decodeMemoryReceipt, memoryErrorMessage, type ContextPreview, type MemoryRecord } from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';

export function MemoryPolicyPanel({ items, epoch, locked }: { items: MemoryRecord[]; epoch: number; locked: boolean }) {
  const [preview,setPreview]=useState<ContextPreview|null>(null);
  const [enabled,setEnabled]=useState(false), [ids,setIds]=useState<string[]>([]);
  const [busy,setBusy]=useState(false), [confirm,setConfirm]=useState(false), [message,setMessage]=useState('');
  const serial=useRef(0), acting=useRef(false);
  useEffect(()=>{
    let live=true;
    async function load(){ const n=++serial.current;
      try { const value=decodeContextPreview(await invoke('chat_context_preview'));
        if(live&&n===serial.current){setPreview(value);setEnabled(value.policy.enabled);setIds(value.policy.selectedIds);setConfirm(false);}
      }catch(e){if(live&&n===serial.current){setPreview(null);setMessage(memoryErrorMessage(e));}}
    }
    if(nativeDesktop)void load();
    const focus=()=>{if(nativeDesktop&&!acting.current)void load();}; window.addEventListener('focus',focus);
    return()=>{live=false;serial.current++;window.removeEventListener('focus',focus);};
  },[epoch]);
  const chosen=enabled?ids:[], chars=chosen.reduce((sum,id)=>sum+[...(items.find(m=>m.id===id)?.body??'')].length,0);
  const removing=preview?.policy.selectedIds.some(id=>!chosen.includes(id))??false;
  async function save(confirmed=false){
    if(!preview||acting.current)return;
    if(removing&&!confirmed){setConfirm(true);return;}
    acting.current=true;setBusy(true);setMessage('');
    const n=++serial.current;
    try {const receipt=decodeMemoryReceipt(await invoke('memory_policy_set',{request:{expectedScope:preview.scope,expectedEpoch:preview.contextEpoch,expectedRevision:preview.policy.revision,enabled,selectedIds:chosen,restartConversation:confirmed}}));
      if(n===serial.current){setConfirm(false);setMessage(`使用设置已保存在本机。${receipt.clearedTurns>0?`已清除使用过所移除条目的最近 ${receipt.clearedTurns} 轮对话，其余保留。`:''}${receipt.notificationsDelivered?'':'另一窗口待刷新。'}`);
        try{const next=decodeContextPreview(await invoke('chat_context_preview'));if(n===serial.current){setPreview(next);setEnabled(next.policy.enabled);setIds(next.policy.selectedIds);}}
        catch{if(n===serial.current){setPreview(null);setMessage('使用设置已保存，但暂时无法刷新；请重新打开面板，不必重复提交。');}}
      }
    }catch(e){if(n===serial.current){setMessage(memoryErrorMessage(e));setConfirm(false);}}
    finally{acting.current=false;setBusy(false);}
  }
  return <section className="memory-policy" aria-label="模型记忆许可">
    <h3>让交流用上这些记忆</h3>
    <p>保存到本机后，还需要单独允许模型使用。每个地址和模型独立设置，新模型默认关闭。</p>
    {preview&&<p className="memory-scope">当前服务：{preview.scope.baseUrl}<br/>模型：{preview.scope.model}<br/>已保存状态：{preview.policy.enabled?`启用 · ${preview.policy.selectedIds.length}条`:'关闭'}</p>}
    <fieldset disabled={!preview||busy||locked||confirm}>
      <label className="memory-choice"><input type="checkbox" checked={enabled} onChange={e=>{setEnabled(e.target.checked);if(!e.target.checked)setIds([]);}}/>允许此模型使用所选记忆</label>
      <p>发送消息时，所选正文、来源和时间将随对话发给上述服务。最多5条、正文合计800字，会增加输入用量。仅保存设置不会调用模型；清空选择后保存将关闭使用。</p>
      {items.map(item=><label className="memory-choice" key={item.id}><input type="checkbox" disabled={!enabled} checked={ids.includes(item.id)} onChange={e=>setIds(old=>e.target.checked?[...old,item.id]:old.filter(id=>id!==item.id))}/><span>{item.body}</span></label>)}
      <small>{chosen.length} / 5 条 · {chars} / 800 字 · 按勾选顺序发送</small>
      <button type="button" disabled={chosen.length>5||chars>800} onClick={()=>void save()}>保存此模型的记忆设置</button>
    </fieldset>
    {confirm&&<div role="alertdialog" aria-label="确认收回记忆使用" className="memory-confirm"><p>停用或移除选择会停止正在生成的回复，清空本机全部模型的聊天。保留本机记忆条目；已经发给服务商的内容不能撤回。</p><button disabled={busy} onClick={()=>void save(true)}>确认收回并清空聊天</button><button disabled={busy} onClick={()=>setConfirm(false)}>返回</button></div>}
    {message&&<p role="status" className="memory-notice">{message}</p>}
  </section>;
}
