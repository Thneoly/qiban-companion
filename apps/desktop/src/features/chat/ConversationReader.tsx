import { useEffect, useRef, useState } from 'react';
import type { ChatTurn } from '@companion/contracts';

export function ConversationReader({ history, prompt, reply, pending, label, waiting }: {
  history: ChatTurn[]; prompt: string; reply: string; pending: boolean; label: string; waiting: boolean;
}) {
  const viewport = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const [away, setAway] = useState(false);
  function bottom() {
    const element = viewport.current;
    if (element) element.scrollTop = element.scrollHeight;
    follow.current = true; setAway(false);
  }
  useEffect(() => { if (follow.current) bottom(); }, [history, prompt, reply]);
  return <div className="chat-reader">
    <div ref={viewport} className="chat-reading-thread" role="region" aria-label="对话阅读区" tabIndex={0}
      onScroll={() => { const e = viewport.current; if (e) { follow.current = e.scrollHeight - e.clientHeight - e.scrollTop < 28; setAway(!follow.current); } }}>
      {history.map((turn, i) => <article className="chat-turn" key={i}>
        <p className="chat-speaker">我</p><p>{turn.user}</p>
        <p className="chat-speaker">栖栖 <span>已加入前文</span></p><p>{turn.assistant}</p>
      </article>)}
      {pending && prompt && <article className="chat-turn chat-turn-pending">
        <p className="chat-speaker">我</p><p>{prompt}</p>
        <p className="chat-speaker">栖栖 <span>{label}</span></p><p>{reply || (waiting ? '正在等待回复…' : '没有收到回复。')}</p>
      </article>}
      {!history.length && !prompt && <p className="chat-reader-empty">从一句话开始。这里只保留本次运行的最近对话。</p>}
    </div>
    {away && <button type="button" className="chat-jump" onClick={bottom}>回到最新 ↓</button>}
  </div>;
}