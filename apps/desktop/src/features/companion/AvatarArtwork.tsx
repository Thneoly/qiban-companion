import { useId } from 'react';
import type { CompanionState } from './presentation';

/** Original code-native prototype artwork, revision 2. No external character assets. */
export function AvatarArtwork({ state = 'idle' }: { state?: CompanionState }) {
  const gradient = useId();
  const fill = `url(#${gradient})`;
  const resting = state === 'quiet' || state === 'paused';
  return (<svg viewBox="0 0 280 310" data-expression={state} role="img" aria-label="白色小精灵栖栖，戴着紫色围巾">
        <defs><linearGradient id={gradient} x1="0" y1="0" x2="1" y2="1"><stop stopColor="#fffefc"/><stop offset="1" stopColor="#e5ddf7"/></linearGradient></defs>
        <ellipse cx="140" cy="285" rx="66" ry="12" fill="#b4a4d0" opacity=".18"/>
        <g className="creature">
          <path d="M76 106Q51 46 83 32Q109 25 116 92M161 91Q172 26 198 40Q222 55 198 113" fill={fill} stroke="#e5dff0" strokeWidth="2"/>
          <path d="M93 86Q79 53 86 47Q97 48 101 87M180 88Q185 53 195 54Q201 60 190 92" fill="#d7c5ee"/>
          <path d="M65 156C63 89 111 74 146 78C209 80 225 121 214 175L222 223Q224 260 185 264L96 264Q54 260 60 221Z" fill={fill} stroke="#e5dff0" strokeWidth="2"/>
          <ellipse cx="89" cy="162" rx="14" ry="8" fill="#edcbd7"/><ellipse cx="193" cy="162" rx="14" ry="8" fill="#edcbd7"/>
          {resting ? <path className="resting-eyes" d="M106 148Q112 152 118 148M165 148Q171 152 177 148" fill="none" stroke="#514363" strokeWidth="3" strokeLinecap="round"/> : state === 'pleased' ? <path d="M106 150Q112 139 118 150M165 150Q171 139 177 150" fill="none" stroke="#514363" strokeWidth="3" strokeLinecap="round"/> : <g className="eyes"><ellipse cx="112" cy="148" rx="5" ry="8" fill="#514363"/><ellipse cx="171" cy="148" rx="5" ry="8" fill="#514363"/></g>}
          {state === 'concerned' && <path d="M105 134L118 129M165 129L178 134" fill="none" stroke="#675078" strokeWidth="3" strokeLinecap="round"/>}
          <path d={state === 'concerned' ? 'M134 169Q141 163 149 169' : resting ? 'M136 166L147 166' : 'M134 164Q141 172 149 164'} fill="none" stroke="#675078" strokeWidth="3" strokeLinecap="round"/>
          <path d="M76 186Q141 207 208 185L210 208Q143 231 73 208Z" fill="#9d85cd"/>
          <path d="M173 205L196 207L187 247Q175 252 165 243Z" fill="#b49adf"/>
          <path d="M66 207Q35 185 37 205Q42 225 65 236" fill="#eee9f6" stroke="#e5dff0" strokeWidth="2"/>
          <path className="companion-hand" d="M211 206Q235 183 240 199Q239 219 218 234" fill="#eee9f6" stroke="#e5dff0" strokeWidth="2"/>
          <ellipse cx="104" cy="266" rx="24" ry="12" fill="#eee8f7"/><ellipse cx="179" cy="266" rx="24" ry="12" fill="#eee8f7"/>
          <path d="M136 224L139 231L147 232L141 237L143 245L136 241L129 245L131 237L125 232L133 231Z" fill="#f5da95"/>
        </g>
        {state === 'thinking' && <g className="thought-dots" fill="#9477b8"><circle cx="228" cy="91" r="4"/><circle cx="243" cy="80" r="5"/><circle cx="260" cy="72" r="6"/></g>}
        {state === 'responding' && <g className="reply-spark" fill="#c9a652"><path d="M244 130L247 138L255 141L247 144L244 152L241 144L233 141L241 138Z"/></g>}
      </svg>);
}
