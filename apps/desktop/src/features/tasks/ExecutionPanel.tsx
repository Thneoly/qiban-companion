import {useEffect,useRef,useState,type FormEvent} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {decodeExecutionDetail,decodeExecutions,type ExecutionDetail,type ExecutionTask,type ExecutionStatus} from '@companion/contracts';
import {nativeDesktop} from '../../lib/surface';
import {errorMessage} from '../../lib/client';
import './execution.css';
const labels:Record<ExecutionStatus,string>={waiting_confirmation:'等待确认保存',running:'正在保存',completed:'已完成',cancelled:'已取消',failed:'未完成',unknown:'结果待核对'};
export function ExecutionPanel(){
  const [expanded,setExpanded]=useState(false);
  return <section className="model-settings document-execution" aria-label="文档摘录任务" id="execution-title">
    <h2>一起整理一份文档</h2><p>选择文本 → 预览摘录 → 确认保存。仅本机处理，不上传、不调用模型。</p>
    <button aria-expanded={expanded} onClick={()=>setExpanded(v=>!v)}>{expanded?'收起文档任务':'打开文档任务'}</button>
    {expanded&&(nativeDesktop?<ExecutionWorkspace/>:<p role="status">请运行桌面版来处理本地文档。浏览器预览不模拟执行成功。</p>)}
  </section>;
}
function ExecutionWorkspace(){
  const [items,setItems]=useState<ExecutionTask[]>([]),[detail,setDetail]=useState<ExecutionDetail|null>(null);
  const [file,setFile]=useState<File|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[result,setResult]=useState<string|null>(null);
  const selectedId=useRef<string|null>(null);
  const requestId=useRef(crypto.randomUUID());const generation=useRef(0);const mounted=useRef(true);
  async function refresh(){
    const token=++generation.current;
    const rows=decodeExecutions(await invoke('execution_list'));
    const current=selectedId.current?decodeExecutionDetail(await invoke('execution_detail',{id:selectedId.current})):null;
    if(mounted.current&&token===generation.current){setItems(rows);if(current)setDetail(current);}
  }
  useEffect(()=>{mounted.current=true;void refresh().catch(e=>setError(errorMessage(e)));return()=>{mounted.current=false;generation.current++;};},[]);
  async function run(command:string,args?:Record<string,unknown>){
    setBusy(true);setError('');setResult(null);generation.current++;
    try{const next=decodeExecutionDetail(await invoke(command,args));if(mounted.current){selectedId.current=next.task.id;setDetail(next);}await refresh();}
    catch(e){if(mounted.current)setError(errorMessage(e));try{await refresh();}catch{/* Keep the original error and prior state. */}}
    finally{if(mounted.current)setBusy(false);}
  }
  async function prepare(event:FormEvent){
    event.preventDefault();if(!file||busy)return;
    if(file.size>256*1024||!file.size||!/^.+\.(txt|md)$/i.test(file.name)){setError('请选择最大 256 KB 的 UTF-8 txt/md 文件。');return;}
    setBusy(true);setError('');
    try{const text=new TextDecoder('utf-8',{fatal:true}).decode(await file.arrayBuffer());await run('execution_prepare',{requestId:requestId.current,sourceName:file.name,text});}
    catch{setError('文件读取失败，请确认编码为 UTF-8。');}finally{setBusy(false);}
  }
  const task=detail?.task;
  return <div className="execution-workspace">
    <form onSubmit={prepare}><label>选择要整理的文档<input type="file" accept=".txt,.md,text/plain,text/markdown" aria-label="选择文档" disabled={busy} onChange={e=>{setFile(e.target.files?.[0]??null);requestId.current=crypto.randomUUID();setError('');}}/></label>
      <button disabled={busy||!file}>生成摘录预览</button></form>
    <p className="execution-help">固定摘录前 8 个非空行，每行最多 240 字；不等同于 AI 摘要。原文件保持不变，摘录与执行记录会在本机保存。</p>
    <button disabled={busy} onClick={()=>{setError('');void refresh().catch(e=>setError(errorMessage(e)));}}>刷新执行记录</button>
    {error&&<p role="alert">{error}</p>}
    {busy&&<p role="status">正在处理，请等待回执；关闭面板不会撤销已经确认的保存。</p>}
    <div className="execution-jobs">{items.map(item=><button key={item.id} disabled={busy} onClick={()=>void run('execution_detail',{id:item.id})} aria-pressed={task?.id===item.id}><strong>{item.sourceName}</strong><span>{labels[item.status]}</span></button>)}{!items.length&&<p>还没有文档执行记录。</p>}</div>
    {task&&<article className="execution-detail" aria-label="当前执行任务">
      <h3>{task.sourceName} · {labels[task.status]}</h3><p role="status">{task.note}</p>
      <pre aria-label="摘录预览">{task.preview}</pre>
      <p>保存位置：应用数据目录中的 document-drafts 文件夹。文件名：<code>{task.artifactName}</code></p>
      {task.status==='waiting_confirmation'&&<div className="execution-actions"><button disabled={busy} onClick={()=>void run('execution_confirm',{id:task.id,expectedRevision:task.revision})}>确认保存草稿</button><button disabled={busy} onClick={()=>void run('execution_cancel',{id:task.id,expectedRevision:task.revision})}>取消保存</button></div>}
      {(task.status==='unknown'||task.status==='running')&&<button disabled={busy} onClick={()=>void run('execution_reconcile',{id:task.id})}>核对实际结果</button>}
      {task.status==='completed'&&<button disabled={busy} onClick={async()=>{setBusy(true);setError('');setResult(null);try{const text:unknown=await invoke('execution_result',{id:task.id});if(typeof text!=='string'||text.length>12000)throw Error('产物内容不兼容');setResult(text);}catch(e){setError(errorMessage(e));}finally{setBusy(false);}}}>读取已保存草稿</button>}
      {result!==null&&<div><h4>已从文件读取并核验</h4><pre aria-label="已保存草稿">{result}</pre></div>}
      <details><summary>执行记录 · {detail.events.length} 条事件</summary><p>任务 {task.id}<br/>动作 {task.actionId}</p><ol>{detail.events.map(e=><li key={e.sequence}>{labels[e.status]} · {new Date(e.createdAt).toLocaleString()}</li>)}</ol><p>执行尝试 {detail.attempts.length} 次</p></details>
    </article>}
  </div>;
}
