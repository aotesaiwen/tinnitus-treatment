/* Only opaque sample IDs reach the listening screen. Source metadata is fetched on reveal. */
const $ = (selector) => document.querySelector(selector);
const audio = $('#audio');
const criteria = [
  { id: 'clarity', name: '声音干净程度', hint: '音色是否清晰，有没有明显杂音' },
  { id: 'music', name: '音乐好听程度', hint: '旋律、节奏和乐句是否自然' },
  { id: 'replay', name: '继续听的意愿', hint: '你有多想把这段音乐继续听下去' },
];
// Persist the existing values so language changes preserve earlier selections.
const artifactLabels = { 'Metallic tones': '金属敲击感', 'Hiss / distortion': '嘶声或失真', 'Unexpected instruments': '混入其他乐器', 'Mechanical rhythm': '节奏机械', 'Abrupt ending': '结尾突兀' };
let manifest, state, storageKey, sources = null, storageAvailable = true;
let changeToken = 0, restorePosition = 0, resumeOnLoad = false, pendingAction = null;
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const timestamp = (t) => `${Math.floor(Math.max(0,t)/60).toString().padStart(2,'0')}:${Math.floor(Math.max(0,t)%60).toString().padStart(2,'0')}`;
const label = (i) => String.fromCharCode(65+i);
const current = () => manifest.samples.find((s) => s.id === state.order[state.index]);
const answer = (id=state.order[state.index]) => state.answers[id] ??= { ratings:{}, artifacts:[], note:'', played:false, skipped:false };
const complete = (id) => { const a=answer(id); return a.skipped || (a.played && criteria.every((c) => Number.isInteger(a.ratings[c.id]) && a.ratings[c.id]>=1 && a.ratings[c.id]<=5)); };

function shuffle(values) {
  const out = [...values];
  for (let i=out.length-1;i>0;i--) {
    const n=i+1, limit=Math.floor(4294967296/n)*n;
    let r; do { r=crypto.getRandomValues(new Uint32Array(1))[0]; } while(r>=limit);
    const j=r%n; [out[i],out[j]]=[out[j],out[i]];
  }
  return out;
}
function freshState() {
  return { version:manifest.version, session:crypto.randomUUID(), createdAt:new Date().toISOString(),
    order:shuffle(manifest.samples.map((s)=>s.id)), index:0, answers:{}, volume:.65, keepPosition:true, revealed:false };
}
function persist() {
  try { localStorage.setItem(storageKey,JSON.stringify(state)); }
  catch { storageAvailable=false; }
}
function loadState() {
  try {
    const s=JSON.parse(localStorage.getItem(storageKey));
    const ids=manifest.samples.map(x=>x.id);
    if(s && s.version===manifest.version && Array.isArray(s.order) && s.order.length===ids.length &&
       new Set(s.order).size===ids.length && s.order.every(x=>ids.includes(x)) &&
       Number.isInteger(s.index) && s.index>=0 && s.index<ids.length && s.answers && typeof s.answers==='object') {
      s.volume=Number.isFinite(s.volume)?Math.max(0,Math.min(1,s.volume)):.65;
      for (const id of s.order) {
        const a=s.answers[id];
        if(!a || !a.ratings || typeof a.ratings!=='object' || !Array.isArray(a.artifacts) || typeof a.note!=='string') delete s.answers[id];
      }
      return s;
    }
  } catch { /* A malformed or stale local session must not break playback. */ }
  return freshState();
}
function renderNav() {
  $('#sample-list').innerHTML=state.order.map((id,i)=>{
    const a=answer(id), ready=complete(id);
    const name=state.revealed && sources? sources.samples[id]?.shortLabel || `片段 ${label(i)}` : `片段 ${label(i)}`;
    const status=a.skipped?'–':ready?'✓':a.played?'·':'';
    return `<button type="button" class="sample-item ${i===state.index?'active':''}" data-index="${i}" aria-current="${i===state.index?'true':'false'}" aria-label="片段 ${label(i)}${a.skipped?'，已跳过':ready?'，已评分':''}"><span class="sample-badge">${label(i)}</span><span class="sample-item-name">${esc(name)}</span><span class="sample-state" aria-hidden="true">${status}</span></button>`;
  }).join('');
  const count=state.order.filter(complete).length;
  $('#progress-count').textContent=`${count} / ${state.order.length}`;
  $('#reveal').disabled=count!==state.order.length || state.revealed;
  $('#finish-hint').textContent=count===state.order.length?'听感已保存，现在可以揭晓每段音乐的来源。':`还有 ${state.order.length-count} 段需要评分或跳过。进度会自动保存。`;
  $('#finish-panel').hidden=state.revealed;
}
function renderAnswer() {
  const a=answer();
  $('#ratings').innerHTML=criteria.map((c)=>`<fieldset class="rating-row" ${state.revealed?'disabled':''}><legend>${c.name}<span class="rating-subtitle">${c.hint}</span></legend><div class="rating-choices">${[1,2,3,4,5].map((n)=>`<label class="rating-choice"><input type="radio" name="${c.id}" value="${n}" ${a.ratings[c.id]===n?'checked':''} aria-label="${c.name}：${n} 分，满分 5 分"><span>${n}</span></label>`).join('')}</div></fieldset>`).join('');
  $('#artifact-options').innerHTML=Object.entries(artifactLabels).map(([value,text])=>`<label class="artifact-option"><input type="checkbox" value="${esc(value)}" ${a.artifacts.includes(value)?'checked':''} ${state.revealed?'disabled':''}><span>${esc(text)}</span></label>`).join('');
  $('#notes').value=a.note; $('#notes').disabled=state.revealed;
  $('#skip').hidden=state.revealed;
  $('#skip').textContent=a.skipped?'撤销跳过，给这段评分':'跳过：暂时无法评价这段';
  $('#next').textContent=state.index===state.order.length-1?'返回片段 A →':'下一段 →';
  updateSaveStatus();
}
function updateSaveStatus() {
  const a=answer();
  $('#save-status').textContent=!storageAvailable?'浏览器无法保存进度，请在关闭页面前导出评分。':state.revealed?'本轮评分已锁定。':a.skipped?'已跳过。揭晓前可以撤销。':!a.played?'先播放这段音乐，再记录听感。':complete(state.order[state.index])?'已保存，可以继续听下一段。':'请为三个项目分别评分。';
}
function syncTransport() {
  const playing=!audio.paused && !audio.ended;
  $('.player-panel').classList.toggle('playing',playing);
  $('#play').innerHTML=`${playing?'暂停':'播放片段'} <span class="play-symbol" aria-hidden="true"></span>`;
  $('#play-state').textContent=playing?(audio.readyState<3?'正在载入音频…':'正在播放'):audio.ended?'已播完，可以再听一次':audio.currentTime>0?'已暂停':'随时开始试听';
}
function select(index, initial=false) {
  if(index<0 || index>=state.order.length) return;
  const wasPlaying=!audio.paused && !audio.ended;
  const at=Number.isFinite(audio.currentTime)?audio.currentTime:0;
  audio.pause(); state.index=index; persist();
  const s=current(); changeToken++;
  restorePosition=!initial && state.keepPosition?Math.min(at,Math.max(0,s.duration-0.1)):0;
  resumeOnLoad=!initial && wasPlaying;
  $('#sample-heading').textContent=state.revealed && sources?sources.samples[s.id]?.label || `片段 ${label(index)}`:`片段 ${label(index)}`;
  $('#sample-letter').textContent=label(index);
  $('#sample-position').textContent=`片段 ${(index+1).toString().padStart(2,'0')} / ${state.order.length.toString().padStart(2,'0')}`;
  $('#player-error').hidden=true; $('#play').disabled=false;
  $('#seek').max=s.duration; $('#seek').value=restorePosition;
  $('#time-current').textContent=timestamp(restorePosition); $('#time-total').textContent=timestamp(s.duration);
  audio.src=s.audio; audio.load();
  renderNav(); renderAnswer(); syncTransport();
}
async function playPause() {
  if(!audio.paused){audio.pause();return;}
  $('#player-error').hidden=true;
  try { await audio.play(); }
  catch { $('#player-error').textContent='这段音乐暂时无法播放，请重试或刷新页面。'; $('#player-error').hidden=false; }
}
audio.addEventListener('loadedmetadata',async()=>{
  const token=changeToken;
  if(Number.isFinite(audio.duration)) {
    audio.currentTime=Math.min(restorePosition,Math.max(0,audio.duration-.05));
    $('#seek').max=audio.duration; $('#time-total').textContent=timestamp(audio.duration);
  }
  if(resumeOnLoad) { resumeOnLoad=false; try{await audio.play();}catch{if(token===changeToken)syncTransport();} }
});
audio.addEventListener('play',()=>{if(!state.revealed){answer().played=true;persist();renderNav();updateSaveStatus();}syncTransport();});
audio.addEventListener('pause',syncTransport);audio.addEventListener('ended',syncTransport);
audio.addEventListener('waiting',syncTransport);audio.addEventListener('playing',syncTransport);
audio.addEventListener('timeupdate',()=>{$('#seek').value=audio.currentTime;$('#time-current').textContent=timestamp(audio.currentTime);});
audio.addEventListener('error',()=>{if(!manifest)return;$('#player-error').textContent='音频载入失败。可以先听其他片段，再回来重试；仍无法播放时可选择跳过。';$('#player-error').hidden=false;syncTransport();});
$('#play').addEventListener('click',playPause);
$('#restart').addEventListener('click',()=>{audio.currentTime=0;if(audio.paused)playPause();});
$('#seek').addEventListener('input',()=>{if(Number.isFinite(audio.duration))audio.currentTime=Number($('#seek').value);});
$('#volume').addEventListener('input',()=>{audio.volume=state.volume=Number($('#volume').value);persist();});
$('#keep-position').addEventListener('change',()=>{state.keepPosition=$('#keep-position').checked;persist();});
$('#sample-list').addEventListener('click',(e)=>{const b=e.target.closest('[data-index]');if(b)select(Number(b.dataset.index));});
$('#next').addEventListener('click',()=>{select((state.index+1)%state.order.length);$('.player-panel').scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth',block:'nearest'});});
$('#ratings').addEventListener('change',(e)=>{if(state.revealed || !e.target.matches('input[type=radio]'))return;answer().ratings[e.target.name]=Number(e.target.value);answer().skipped=false;persist();renderNav();updateSaveStatus();$('#skip').textContent='跳过：暂时无法评价这段';});
$('#artifact-options').addEventListener('change',()=>{if(state.revealed)return;answer().artifacts=[...$('#artifact-options').querySelectorAll('input:checked')].map(x=>x.value);persist();updateSaveStatus();});
$('#notes').addEventListener('input',()=>{if(state.revealed)return;answer().note=$('#notes').value;persist();updateSaveStatus();});
$('#skip').addEventListener('click',()=>{answer().skipped=!answer().skipped;persist();renderNav();renderAnswer();});
document.addEventListener('keydown',(e)=>{if(!manifest || !state || e.target.closest('input,textarea,button,a,dialog,select'))return;if(e.code==='Space'){e.preventDefault();playPause();}});

function askConfirmation(action) {
  pendingAction=action;
  const reset=action==='reset';
  $('#confirm-title').textContent=reset?'开始新一轮试听吗？':'现在揭晓来源吗？';
  $('#confirm-text').textContent=reset?'新一轮会清空当前评分，如需保留请先导出。音乐片段相同，只会重新打乱顺序；已经知道的来源无法恢复为未知。':'揭晓后，本轮评分将锁定；你仍然可以继续试听。';
  $('#confirm-action').textContent=reset?'开始新一轮':'揭晓结果';
  $('#confirm-dialog').showModal();
}
$('#reveal').addEventListener('click',()=>askConfirmation('reveal'));
$('#new-session').addEventListener('click',()=>askConfirmation('reset'));
$('#confirm-dialog').addEventListener('close',async()=>{
  if($('#confirm-dialog').returnValue!=='confirm')return;
  if(pendingAction==='reset') {
    audio.pause();state=freshState();sources=null;persist();audio.volume=state.volume;$('#volume').value=state.volume;$('#keep-position').checked=state.keepPosition;document.body.classList.remove('revealed');$('#results').hidden=true;select(0,true);$('#finish-panel').hidden=false;return;
  }
  $('#reveal').disabled=true;
  try {
    const r=await fetch('reveal.json',{cache:'no-store'});if(!r.ok)throw new Error();
    const data=await r.json();
    if(data.version!==manifest.version || !state.order.every(id=>data.samples[id]))throw new Error();
    sources=data;state.revealed=true;state.revealedAt=new Date().toISOString();persist();renderResults();renderNav();renderAnswer();
    $('#sample-heading').textContent=sources.samples[current().id].label;
    $('#results').scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});
  } catch {$('#global-error').textContent='来源信息暂时无法载入。评分已保留，请再次尝试揭晓。';$('#global-error').hidden=false;renderNav();}
});
function renderResults() {
  document.body.classList.add('revealed');$('#results').hidden=false;$('#global-error').hidden=true;
  const rows=state.order.map((id,i)=>({id,index:i,a:answer(id),source:sources.samples[id]}));
  rows.sort((x,y)=>(y.a.skipped?-1:y.a.ratings.replay||0)-(x.a.skipped?-1:x.a.ratings.replay||0));
  $('#results-list').innerHTML=rows.map(({index,a,source})=>`<article class="result-row"><div class="result-letter">${label(index)}</div><div><h3 class="result-source">${esc(source.label)}</h3><p class="result-detail">${esc(source.description)}</p><p class="result-detail">${esc(source.configuration || '')}</p>${source.url?`<p class="result-detail"><a href="${esc(source.url)}" target="_blank" rel="noopener noreferrer">来源与说明 ↗</a></p>`:''}${a.note?`<p class="result-note">${esc(a.note)}</p>`:''}${a.artifacts.length?`<p class="result-detail">记录的问题：${a.artifacts.map(value=>esc(artifactLabels[value] || value)).join(' · ')}</p>`:''}</div><div class="result-score">${a.skipped?'已跳过':criteria.map(c=>`${c.name}<b>${a.ratings[c.id]??'–'} / 5</b><br>`).join('')}</div></article>`).join('');
}
$('#export').addEventListener('click',()=>{
  const result={schema:'listening-room-feedback/v1',dataset:manifest.version,session:state.session,createdAt:state.createdAt,revealedAt:state.revealedAt,method:manifest.method,samples:state.order.map((id,i)=>({anonymousLabel:label(i),id,...answer(id),source:sources?.samples[id]??null}))};
  const url=URL.createObjectURL(new Blob([JSON.stringify(result,null,2)],{type:'application/json'}));
  const a=document.createElement('a');a.href=url;a.download=`listening-room-${state.session.slice(0,8)}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
});
function showCoverage() {
  $('#coverage-content').innerHTML=`<p>${esc(manifest.method)}</p><p>${esc(manifest.coverageNote)}</p>${manifest.coverage.map(c=>`<div class="coverage-row"><strong>${esc(c.name)}</strong><span class="availability">${esc(c.status)}</span><p>${esc(c.detail)}</p></div>`).join('')}<p>试听片段均来自实际生成的音频或明确标注的对照。未测试的模型没有对应试听片段。</p><p>试听时只显示匿名编号，点击揭晓后才载入来源说明。本页用于个人盲听；主动检查网站文件仍可能看到来源。</p><p>${esc(manifest.attribution || '')}</p>`;
  $('#coverage-dialog').showModal();
}
document.querySelectorAll('.coverage-trigger').forEach(b=>b.addEventListener('click',showCoverage));
$('#close-coverage').addEventListener('click',()=>$('#coverage-dialog').close());

async function init() {
  try {
    const r=await fetch('manifest.json',{cache:'no-store'});if(!r.ok)throw new Error();manifest=await r.json();
    if(!Array.isArray(manifest.samples) || manifest.samples.length===0){$('#session-meta').textContent='音频正在准备中';$('#global-error').textContent='试听页面已就绪，实际生成的音频仍在准备中。';$('#global-error').hidden=false;return;}
    storageKey=`listening-room:${manifest.version}`;state=loadState();persist();
    audio.volume=state.volume;$('#volume').value=state.volume;$('#keep-position').checked=state.keepPosition;
    $('#session-meta').textContent=`${manifest.samples.length} 段音乐 · 每段 ${manifest.samples[0].duration} 秒 · 统一响度`;
    $('#listening').hidden=false;$('#finish-panel').hidden=false;select(state.index,true);
    if(state.revealed) {
      try {const rr=await fetch('reveal.json',{cache:'no-store'});if(!rr.ok)throw new Error();const data=await rr.json();if(data.version!==manifest.version)throw new Error();sources=data;renderResults();renderNav();$('#sample-heading').textContent=sources.samples[current().id].label;}
      catch{$('#global-error').textContent='已保存的评分处于锁定状态，但来源说明载入失败。请刷新页面重试。';$('#global-error').hidden=false;}
    }
  } catch {$('#global-error').textContent='试听内容载入失败，请刷新页面。';$('#global-error').hidden=false;$('#session-meta').textContent='暂时无法载入';}
}
init();
