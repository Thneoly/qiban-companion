import {useEffect,useState,type FormEvent} from 'react';
import {listen} from '@tauri-apps/api/event';
import {invoke} from '@tauri-apps/api/core';
import {nativeDesktop} from '../../lib/surface';
import {decodeModelSettings,type ModelSettingsConfig} from '@companion/contracts';
const initial:ModelSettingsConfig={baseUrl:'https://open.bigmodel.cn/api/paas/v4',model:'glm-5.3',useApiKey:true,hasApiKey:false};
export function ModelSettings() {
  const [saved,setSaved]=useState(initial);
  const [draft,setDraft]=useState(initial);
  const [busy,setBusy]=useState(false);
  const [note,setNote]=useState(nativeDesktop?'正在读取设置…':'浏览器仅预览；请在桌面版保存模型与密钥。');
  const dirty=draft.baseUrl!==saved.baseUrl||draft.model!==saved.model||draft.useApiKey!==saved.useApiKey;
  async function refresh() {
    const value=decodeModelSettings(await invoke('model_settings_get'));setSaved(value);setDraft(value);return value;
  }
  useEffect(()=>{if(nativeDesktop)void refresh().then(()=>setNote('支持兼容 OpenAI Chat Completions 的服务。')).catch(()=>setNote('读取设置失败'));},[]);
  useEffect(()=>{let disposed=false;let remove:(()=>void)|undefined;if(nativeDesktop)void listen('model-settings-open',()=>document.getElementById('model-settings-title')?.scrollIntoView()).then(fn=>{if(disposed)fn();else remove=fn;}).catch(()=>setNote('设置事件连接失败'));return ()=>{disposed=true;remove?.();};},[]);
  async function save(event:FormEvent) {
    event.preventDefault();setBusy(true);
    try{await invoke('model_settings_save',{config:{baseUrl:draft.baseUrl,model:draft.model,useApiKey:draft.useApiKey}});await refresh();setNote('已保存。重新打开角色气泡后，新对话使用此配置。');}
    catch(e){setNote(typeof e==='string'?e:'保存失败');}finally{setBusy(false);}
  }
  async function key(action:'model_key_set'|'model_key_delete') {
    setBusy(true);
    try{await invoke(action);await refresh();setNote(action==='model_key_set'?'密钥已保存到 Windows 凭据管理器。':'当前 API 地址的密钥已删除。');}
    catch(e){setNote(typeof e==='string'?e:'密钥操作失败');}finally{setBusy(false);}
  }
  return <section className="model-settings" aria-labelledby="model-settings-title">
    <h2 id="model-settings-title">模型设置</h2>
    <p>填写服务商提供的 API 基地址与模型编码。发送对话时，内容会传给这里配置的服务。</p>
    <form onSubmit={save}>
      <fieldset disabled={busy||!nativeDesktop}>
        <div className="model-presets"><button type="button" onClick={()=>setDraft({...initial})}>智谱通用 API 预设</button><span>也可直接填写其他兼容服务</span></div>
        <label>API 基地址<input value={draft.baseUrl} onChange={e=>setDraft({...draft,baseUrl:e.target.value})} placeholder="https://your-provider.example/v1" maxLength={512}/></label>
        <label>模型编码<input value={draft.model} onChange={e=>setDraft({...draft,model:e.target.value})} maxLength={160}/></label>
        <label className="model-key-option"><input type="checkbox" checked={draft.useApiKey} onChange={e=>setDraft({...draft,useApiKey:e.target.checked})}/>此服务需要 API Key</label>
        <button type="submit">保存模型设置</button>
        <div className="model-key-controls"><span>{saved.hasApiKey?'此 API 地址已保存密钥':'此 API 地址尚未保存密钥'}</span>
          <button type="button" disabled={dirty} onClick={()=>void key('model_key_set')}>设置 / 更换 API Key</button>
          <button type="button" disabled={dirty||!saved.hasApiKey} onClick={()=>void key('model_key_delete')}>删除密钥</button>
        </div>
      </fieldset>
    </form>
    <p className="model-settings-note" role="status">{note}</p>
    <small>先保存地址，再配置密钥。密钥在 Windows 原生窗口的密码栏输入，不回传给网页；不同 API 地址分别保存。</small>
  </section>;
}
