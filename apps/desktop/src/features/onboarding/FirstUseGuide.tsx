import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { decodeChatConfig } from '@companion/contracts';
import { nativeDesktop } from '../../lib/surface';

export function FirstUseGuide({ onComplete, onLater, onSettings, initialError }: {
  onComplete: () => Promise<void>; onLater: () => void; onSettings: () => void; initialError: string;
}) {
  const [configuration, setConfiguration] = useState(nativeDesktop ? '正在查看模型设置…' : '浏览器仅预览，聊天请使用桌面版。');
  const [error, setError] = useState(initialError);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let disposed = false;
    if (nativeDesktop) void invoke('chat_config').then(value => {
      const config = decodeChatConfig(value);
      if (!disposed) setConfiguration(config.configured ? `已配置 ${config.model}；是否可用需由你发送第一句话验证。` : '聊天还需要模型配置；也可以先记待办。');
    }).catch(() => { if (!disposed) setConfiguration('暂时读不到模型设置，可稍后在“模型设置”检查。'); });
    return () => { disposed = true; };
  }, []);
  async function finish() {
    setBusy(true); setError('');
    try { await onComplete(); }
    catch (e) { setError(typeof e === 'string' ? e : '未能记住完成状态，可重试或先随便看看。'); }
    finally { setBusy(false); }
  }
  return <div className="first-use-guide" aria-label="初次使用指南">
    <div className="guide-content">
    <h2>初次见面，我是栖栖</h2>
    <p>我会待在桌面一角，陪你聊聊，也帮你记下想做的事。</p>
    <ol>
      <li><strong>从一句话开始</strong><br/>在“模型设置”填写服务地址、模型和密钥，再回到“聊一聊”。发送时，内容和当前模型前文会交给该服务。</li>
      <li><strong>记录留在这台电脑</strong><br/>完整问答明文保存，重开可恢复；所有模型共6轮／1.2万字。“清空对话”删除本机全部模型记录，不删除服务商留存。待办目前只记录，不执行。</li>
      <li><strong>需要时找得到我</strong><br/>拖动底部把手移动；Esc收起。安静或隐藏后，点任务栏托盘图标唤回；右键可找回角色、退出栖伴。托盘图标可能在“^”里。</li>
    </ol>
    <p className="guide-configuration">{configuration}</p>
    {error && <p role="alert">{error}</p>}
    </div>
    <div className="guide-buttons">
      <button type="button" disabled={busy} onClick={onSettings}>去设置模型</button>
      <button type="button" disabled={busy} onClick={() => void finish()}>{busy ? '正在记住…' : '知道了，开始相处'}</button>
      <button type="button" disabled={busy} onClick={onLater}>先随便看看</button>
    </div>
    <small>可随时从气泡的“使用指南”重看。不会自动发送消息或申请麦克风。</small>
  </div>;
}
