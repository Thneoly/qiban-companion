import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { nativeDesktop } from '../../lib/surface';
import { errorMessage } from '../../lib/client';

type Reminder = {
  id: string;
  dueAt: number;
  message: string;
  status: 'pending' | 'fired' | 'cancelled';
};

const formatDue = (dueAt: number) => {
  const date = new Date(dueAt);
  return `${date.getHours().toString().padStart(2, '0')}:${date.getMinutes().toString().padStart(2, '0')}`;
};

/** App-local reminders: user-created, time-fired, in-app only. The 提醒到点
 * shows as a pet-window bubble; no OS notification is claimed or sent. */
export function RemindersPanel() {
  const [items, setItems] = useState<Reminder[] | null>(null);
  const [message, setMessage] = useState('');
  const [minutes, setMinutes] = useState('10');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const refresh = useCallback(() => {
    if (!nativeDesktop) return;
    invoke<Reminder[]>('reminder_list')
      .then(setItems)
      .catch(e => setError(errorMessage(e)));
  }, []);

  useEffect(() => {
    refresh();
    if (!nativeDesktop) return;
    let disposed = false;
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen('panel-refresh', () => { if (!disposed) refresh(); }).then(unlisten => {
        if (disposed) unlisten();
      }));
    return () => { disposed = true; };
  }, [refresh]);

  const add = async () => {
    const trimmed = message.trim();
    const parsed = Number(minutes);
    if (!trimmed || !Number.isInteger(parsed) || parsed < 1 || parsed > 1440) {
      setError('提醒内容需要 1～200 字，时间在 1 分钟到 24 小时之间。');
      return;
    }
    setBusy(true); setError('');
    try {
      setItems(await invoke<Reminder[]>('reminder_add', { message: trimmed, dueInMinutes: parsed }));
      setMessage('');
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const cancel = async (id: string) => {
    setBusy(true); setError('');
    try {
      setItems(await invoke<Reminder[]>('reminder_cancel', { id }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (!nativeDesktop) return null;

  return (
    <section className="companion-card" aria-label="应用内提醒">
      <h2>提醒</h2>
      <p className="memory-help">由你在本机设置；到点后栖栖在桌面提醒一次，不发送系统通知。</p>
      <div className="reminder-form">
        <label>
          内容
          <input value={message} maxLength={200} onChange={e => setMessage(e.target.value)} placeholder="比如：休息一下，看看窗外" />
        </label>
        <label>
          几分钟后
          <input value={minutes} inputMode="numeric" style={{ width: '5em' }} onChange={e => setMinutes(e.target.value)} />
        </label>
        <button type="button" disabled={busy} onClick={() => void add()}>设置提醒</button>
      </div>
      {error && <p role="alert">{error}</p>}
      {items?.length ? (
        <ul className="reminder-list">
          {items.map(item => (
            <li key={item.id} className={item.status}>
              <span className="reminder-due">{formatDue(item.dueAt)}</span>
              <span className="reminder-text">{item.message}</span>
              <span className="reminder-status">
                {item.status === 'pending' ? '待提醒' : item.status === 'fired' ? '已提醒' : '已取消'}
              </span>
              {item.status === 'pending' && (
                <button type="button" disabled={busy} onClick={() => void cancel(item.id)}>取消</button>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="memory-help">还没有提醒。</p>
      )}
    </section>
  );
}
