import {useEffect,useState,type FormEvent} from 'react';
import {listen} from '@tauri-apps/api/event';
import {invoke} from '@tauri-apps/api/core';
import {nativeDesktop} from '../../lib/surface';
import {decodeModelSettings,decodeModelTestOutcome,type ModelSettingsConfig,type ModelTestOutcome} from '@companion/contracts';
const initial:ModelSettingsConfig={baseUrl:'https://open.bigmodel.cn/api/paas/v4',model:'glm-5.3',useApiKey:true,hasApiKey:false,maxOutputTokens:1024,protocol:'openai'};
const anthropic:ModelSettingsConfig={baseUrl:'https://api.anthropic.com/v1',model:'claude-sonnet-4-5',useApiKey:true,hasApiKey:false,maxOutputTokens:1024,protocol:'anthropic'};
export function ModelSettings() {
  const [saved,setSaved]=useState(initial);
  const [draft,setDraft]=useState(initial);
  const [budget,setBudget]=useState('1024');
  const [busy,setBusy]=useState(false);
  const [testing,setTesting]=useState(false);
  const [testResult,setTestResult]=useState<ModelTestOutcome|null>(null);
  const [note,setNote]=useState(nativeDesktop?'正在读取设置…':'浏览器仅预览；请在桌面版保存模型与密钥。');
  const dirty=draft.baseUrl!==saved.baseUrl||draft.model!==saved.model||draft.useApiKey!==saved.useApiKey||budget!==String(saved.maxOutputTokens);
  async function refresh() {
    const value=decodeModelSettings(await invoke('model_settings_get'));setSaved(value);setDraft(value);setBudget(String(value.maxOutputTokens));return value;
  }
  useEffect(()=>{if(nativeDesktop)void refresh().then(()=>setNote('支持兼容 OpenAI Chat Completions 的服务。')).catch(()=>setNote('读取设置失败'));},[]);
  useEffect(()=>{let disposed=false;let remove:(()=>void)|undefined;if(nativeDesktop)void listen('model-settings-open',()=>document.getElementById('model-settings-title')?.scrollIntoView()).then(fn=>{if(disposed)fn();else remove=fn;}).catch(()=>setNote('设置事件连接失败'));return ()=>{disposed=true;remove?.();};},[]);
  async function save(event:FormEvent) {
    event.preventDefault();
    const maxOutputTokens=Number(budget);
    if(!Number.isInteger(maxOutputTokens)||maxOutputTokens<128||maxOutputTokens>8192){setNote("最大输出 tokens 需要是128～8192之间的整数");return;}
    setBusy(true);
    try{await invoke('model_settings_save',{config:{baseUrl:draft.baseUrl,model:draft.model,useApiKey:draft.useApiKey,protocol:draft.protocol,maxOutputTokens}});await refresh();setNote('已保存。重新打开角色气泡后，新对话使用此配置；记录按地址和模型隔离，切换不会把旧前文发送给新服务。');}
    catch(e){setNote(typeof e==='string'?e:'保存失败');}finally{setBusy(false);}
  }
  async function key(action:'model_key_set'|'model_key_delete') {
    setBusy(true);
    try{await invoke(action);await refresh();setNote(action==='model_key_set'?'密钥已保存到 Windows 凭据管理器。':'当前 API 地址的密钥已删除。');}
    catch(e){setNote(typeof e==='string'?e:'密钥操作失败');}finally{setBusy(false);}
  }
  // Probe the DRAFT values without saving: one minimal 1-token request. The
  // key is looked up by address, so testing a NEW address requires saving it
  // and setting its key first — the outcome text says so when relevant.
  async function testConnection() {
    setTesting(true);setTestResult(null);
    try{setTestResult(decodeModelTestOutcome(await invoke('model_settings_test',{config:{baseUrl:draft.baseUrl,model:draft.model,useApiKey:draft.useApiKey,protocol:draft.protocol}})));}
    catch(e){setTestResult({ok:false,detail:typeof e==='string'?e:'测试请求失败，请稍后重试'});}
    finally{setTesting(false);}
  }
  return <section className="model-settings" aria-labelledby="model-settings-title">
    <h2 id="model-settings-title">模型设置</h2>
    <p>填写服务商提供的 API 基地址与模型编码。发送对话时，内容会传给这里配置的服务。</p>
    <form onSubmit={save}>
      <fieldset disabled={busy||!nativeDesktop}>
        <div className="model-presets"><button type="button" onClick={()=>{setDraft({...initial});setBudget(String(initial.maxOutputTokens));}}>智谱通用 API 预设</button><button type="button" onClick={()=>{setDraft({...anthropic});setBudget(String(anthropic.maxOutputTokens));}}>Anthropic 预设</button><span>也可直接填写其他兼容服务</span></div>
        <label>兼容协议<select value={draft.protocol} onChange={e=>setDraft({...draft,protocol:e.target.value as ModelSettingsConfig['protocol']})}><option value="openai">OpenAI Chat Completions</option><option value="anthropic">Anthropic Messages</option></select></label>
        <label>API 基地址<input value={draft.baseUrl} onChange={e=>setDraft({...draft,baseUrl:e.target.value})} placeholder="https://your-provider.example/v1" maxLength={512}/></label>
        <label>模型编码<input value={draft.model} onChange={e=>setDraft({...draft,model:e.target.value})} maxLength={160}/></label>
        <label>最大输出 tokens<input type="number" required min={128} max={8192} step={1} value={budget} onChange={e=>setBudget(e.target.value)} aria-describedby="output-budget-help"/></label>
        <small id="output-budget-help">128～8192，默认1024。上限不是实际用量；调高可能增加等待和消耗，仍受服务商限制。</small>
        <label className="model-key-option"><input type="checkbox" checked={draft.useApiKey} onChange={e=>setDraft({...draft,useApiKey:e.target.checked})}/>此服务需要 API Key</label>
        <button type="submit">保存模型设置</button>
        <button type="button" disabled={testing||!nativeDesktop} onClick={()=>void testConnection()}>{testing?'正在测试…':'测试连接'}</button>
        {testResult&&<p role="status" className={testResult.ok?'model-test-ok':'model-test-fail'}>{testResult.ok?'✓ ':'✗ '}{testResult.detail}</p>}
        <div className="model-key-controls"><span>{saved.hasApiKey?'此 API 地址已保存密钥':'此 API 地址尚未保存密钥'}</span>
          <button type="button" disabled={dirty} onClick={()=>void key('model_key_set')}>设置 / 更换 API Key</button>
          <button type="button" disabled={dirty||!saved.hasApiKey} onClick={()=>void key('model_key_delete')}>删除密钥</button>
        </div>
      </fieldset>
    </form>
    <p className="model-settings-note" role="status">{note}</p>
    <small>先保存地址，再配置密钥。密钥在 Windows 原生窗口的密码栏输入，不回传给网页；不同 API 地址分别保存。「测试连接」用表单当前地址与模型发一条 1 token 的最小请求；换了地址要先保存并设置密钥再测。</small>
  </section>;
}
