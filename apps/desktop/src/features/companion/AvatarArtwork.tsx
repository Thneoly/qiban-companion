/** Shared prototype artwork: no window or interaction state. */
export function AvatarArtwork() {
  return (<svg viewBox="0 0 280 310" role="img" aria-label="白色小精灵栖栖，戴着紫色围巾">
        <defs><linearGradient id="body" x1="0" y1="0" x2="1" y2="1"><stop stopColor="#fffefc"/><stop offset="1" stopColor="#e5ddf7"/></linearGradient></defs>
        <ellipse cx="140" cy="285" rx="66" ry="12" fill="#b4a4d0" opacity=".18"/>
        <g className="creature">
          <path d="M76 106Q51 46 83 32Q109 25 116 92M161 91Q172 26 198 40Q222 55 198 113" fill="url(#body)" stroke="#e5dff0" strokeWidth="2"/>
          <path d="M93 86Q79 53 86 47Q97 48 101 87M180 88Q185 53 195 54Q201 60 190 92" fill="#d7c5ee"/>
          <path d="M65 156C63 89 111 74 146 78C209 80 225 121 214 175L222 223Q224 260 185 264L96 264Q54 260 60 221Z" fill="url(#body)" stroke="#e5dff0" strokeWidth="2"/>
          <ellipse cx="89" cy="162" rx="14" ry="8" fill="#edcbd7"/><ellipse cx="193" cy="162" rx="14" ry="8" fill="#edcbd7"/>
          <g className="eyes"><ellipse cx="112" cy="148" rx="5" ry="8" fill="#514363"/><ellipse cx="171" cy="148" rx="5" ry="8" fill="#514363"/></g>
          <path d="M134 164Q141 172 149 164" fill="none" stroke="#675078" strokeWidth="3" strokeLinecap="round"/>
          <path d="M76 186Q141 207 208 185L210 208Q143 231 73 208Z" fill="#9d85cd"/>
          <path d="M173 205L196 207L187 247Q175 252 165 243Z" fill="#b49adf"/>
          <path d="M66 207Q35 185 37 205Q42 225 65 236M211 206Q235 183 240 199Q239 219 218 234" fill="#eee9f6" stroke="#e5dff0" strokeWidth="2"/>
          <ellipse cx="104" cy="266" rx="24" ry="12" fill="#eee8f7"/><ellipse cx="179" cy="266" rx="24" ry="12" fill="#eee8f7"/>
          <path d="M136 224L139 231L147 232L141 237L143 245L136 241L129 245L131 237L125 232L133 231Z" fill="#f5da95"/>
        </g>
      </svg>);
}
