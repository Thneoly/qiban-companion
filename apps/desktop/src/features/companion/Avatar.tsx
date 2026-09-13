import { AvatarArtwork } from './AvatarArtwork';
import { useEffect, useRef, useState } from 'react';

/** Code-native prototype asset. Replace this renderer with Live2D/VRM behind the same state boundary. */
export function Avatar() {
  const [greeting, setGreeting] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  function greet() {
    clearTimeout(timer.current);
    setGreeting(true);
    timer.current = setTimeout(() => setGreeting(false), 2200);
  }
  return <div className="avatar-scene">
    <div className="orb orb-one" /><div className="orb orb-two" /><span className="spark spark-one">✧</span><span className="spark spark-two">✦</span>
    <div className="avatar-bubble" aria-live="polite">{greeting ? '收到你的招呼啦。' : '慢慢来，我在这里。'}</div>
    <button className={`avatar-button ${greeting ? 'greeting' : ''}`} onClick={greet} aria-label="和栖栖打个招呼">
      <AvatarArtwork/>
    </button>
    <div className="avatar-caption">点击栖栖，打个招呼 <span>· 原型角色</span></div>
  </div>;
}
